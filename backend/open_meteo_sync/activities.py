"""Temporal activity for the open-meteo sync.

Open-meteo is a third-party Swift binary (`ghcr.io/open-meteo/open-meteo`),
not our code. The "Temporal-native" pattern for orchestrating an external
binary you can't import is: have an activity create a Kubernetes Job that
runs the binary, then poll the Job until it terminates.

  - Activity is idempotent in the Temporal-retry sense: each invocation
    creates a freshly-named Job (suffix = workflow run id + attempt) so a
    retry doesn't collide with a still-running Job from the previous attempt.
  - Heartbeats every 30s while the Job is in progress.
  - Returns the final Job status; non-zero exit code → ApplicationError so
    the workflow surfaces failure.
  - Runs in-cluster: uses the worker pod's ServiceAccount + RBAC.
    See deploy/k8s/rbac-temporal-worker.yaml for the required Role.
"""

from __future__ import annotations

import asyncio
import os
import time
from dataclasses import dataclass, field
from typing import Any

from temporalio import activity
from temporalio.exceptions import ApplicationError

from backend.shared.logger import get_logger


log = get_logger("open-meteo-sync-activities")


NAMESPACE = os.environ.get("OPEN_METEO_SYNC_NAMESPACE", "radar-ng")
PVC_NAME = os.environ.get("OPEN_METEO_PVC", "openmeteo-data")
IMAGE = os.environ.get("OPEN_METEO_IMAGE", "ghcr.io/open-meteo/open-meteo:latest")
JOB_TIMEOUT_S = int(os.environ.get("OPEN_METEO_JOB_TIMEOUT_S", "1800"))


@dataclass
class OpenMeteoSyncArgs:
    model: str            # "ncep_gfs025" | "ncep_hrrr_conus"
    variables: str        # comma-separated list passed as the third arg
    past_days: int = 1    # --past-days


@dataclass
class OpenMeteoSyncResult:
    job_name: str
    succeeded: bool
    duration_s: float
    log_tail: str = ""
    failure_reason: str = ""


def _build_job_manifest(name: str, args: OpenMeteoSyncArgs) -> dict[str, Any]:
    """Same shape as the legacy CronJob's jobTemplate."""
    return {
        "apiVersion": "batch/v1",
        "kind": "Job",
        "metadata": {"name": name, "namespace": NAMESPACE, "labels": {"app": "open-meteo-sync", "model": args.model}},
        "spec": {
            "ttlSecondsAfterFinished": 300,
            "backoffLimit": 0,  # Temporal retries; k8s shouldn't double-retry
            "template": {
                "spec": {
                    "restartPolicy": "Never",
                    "securityContext": {
                        "runAsNonRoot": True,
                        "runAsUser": 999,
                        "runAsGroup": 999,
                        "fsGroup": 999,
                        "fsGroupChangePolicy": "OnRootMismatch",
                        "seccompProfile": {"type": "RuntimeDefault"},
                    },
                    "containers": [{
                        "name": "sync",
                        "image": IMAGE,
                        "securityContext": {
                            "allowPrivilegeEscalation": False,
                            "capabilities": {"drop": ["ALL"]},
                        },
                        "args": ["sync", args.model, args.variables, "--past-days", str(args.past_days)],
                        "volumeMounts": [{"name": "openmeteo-data", "mountPath": "/app/data"}],
                        "resources": {
                            "requests": {"cpu": "200m", "memory": "512Mi"},
                            "limits":   {"cpu": "2",    "memory": "4Gi"},
                        },
                    }],
                    "volumes": [{
                        "name": "openmeteo-data",
                        "persistentVolumeClaim": {"claimName": PVC_NAME},
                    }],
                }
            },
        },
    }


@activity.defn(name="open_meteo_sync_via_k8s_job")
async def open_meteo_sync_via_k8s_job(args: OpenMeteoSyncArgs) -> OpenMeteoSyncResult:
    """Create a k8s Job, watch it to completion, return outcome."""
    from kubernetes import client, config, watch  # type: ignore

    info = activity.info()
    # Job name must be DNS-1123 (≤63 chars). Truncate run-id to 8 chars.
    job_name = f"om-{args.model.replace('_', '-')}-{info.workflow_run_id[:8]}-{info.attempt}"
    started = time.time()

    def _setup() -> tuple[Any, Any]:
        try:
            config.load_incluster_config()
        except Exception:
            config.load_kube_config()  # local dev fallback
        return client.BatchV1Api(), client.CoreV1Api()

    batch, core = await asyncio.to_thread(_setup)

    manifest = _build_job_manifest(job_name, args)

    def _create() -> None:
        try:
            batch.create_namespaced_job(namespace=NAMESPACE, body=manifest)
        except client.rest.ApiException as e:  # type: ignore[attr-defined]
            if e.status == 409:
                log.info("job_exists_reusing", extra={"job": job_name})
            else:
                raise

    await asyncio.to_thread(_create)
    log.info("job_created", extra={"job": job_name, "model": args.model})

    deadline = time.monotonic() + JOB_TIMEOUT_S
    last_heartbeat = 0.0
    while time.monotonic() < deadline:
        def _read() -> Any:
            return batch.read_namespaced_job_status(name=job_name, namespace=NAMESPACE)

        job = await asyncio.to_thread(_read)
        status = job.status
        if status.succeeded:
            duration = time.time() - started
            tail = await asyncio.to_thread(_tail_logs, core, job_name)
            return OpenMeteoSyncResult(job_name=job_name, succeeded=True, duration_s=round(duration, 1), log_tail=tail)
        if status.failed:
            duration = time.time() - started
            tail = await asyncio.to_thread(_tail_logs, core, job_name)
            raise ApplicationError(
                f"open-meteo job {job_name} failed: {tail[-500:]}",
                non_retryable=False,
            )

        now = time.monotonic()
        if now - last_heartbeat >= 30:
            activity.heartbeat({"job": job_name, "active": status.active or 0, "elapsed_s": int(time.time() - started)})
            last_heartbeat = now

        await asyncio.sleep(10)

    raise ApplicationError(f"open-meteo job {job_name} exceeded {JOB_TIMEOUT_S}s timeout", non_retryable=False)


def _tail_logs(core: Any, job_name: str) -> str:
    from kubernetes import client  # type: ignore
    try:
        pods = core.list_namespaced_pod(namespace=NAMESPACE, label_selector=f"job-name={job_name}")
        if not pods.items:
            return ""
        pod = pods.items[0]
        return core.read_namespaced_pod_log(name=pod.metadata.name, namespace=NAMESPACE, tail_lines=50) or ""
    except client.rest.ApiException:  # type: ignore[attr-defined]
        return ""
    except Exception:  # noqa: BLE001
        return ""
