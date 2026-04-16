"""Structured JSON logger + retry-with-backoff helper shared by ingestors."""

from __future__ import annotations

import json
import logging
import os
import sys
import time
from functools import wraps
from typing import Any, Callable, TypeVar


class JsonFormatter(logging.Formatter):
    """One-line JSON per log record."""

    def format(self, record: logging.LogRecord) -> str:
        payload: dict[str, Any] = {
            "ts": self.formatTime(record, "%Y-%m-%dT%H:%M:%S%z"),
            "level": record.levelname,
            "service": record.name,
            "msg": record.getMessage(),
        }
        if record.exc_info:
            payload["exc"] = self.formatException(record.exc_info)
        for key, value in record.__dict__.items():
            if key.startswith("_") or key in (
                "args", "asctime", "created", "exc_info", "exc_text",
                "filename", "funcName", "levelname", "levelno", "lineno",
                "message", "module", "msecs", "msg", "name", "pathname",
                "process", "processName", "relativeCreated", "stack_info",
                "thread", "threadName", "taskName",
            ):
                continue
            payload[key] = value
        return json.dumps(payload, default=str)


def get_logger(service: str) -> logging.Logger:
    logger = logging.getLogger(service)
    if getattr(logger, "_stormscope_configured", False):
        return logger
    handler = logging.StreamHandler(stream=sys.stdout)
    handler.setFormatter(JsonFormatter())
    logger.handlers.clear()
    logger.addHandler(handler)
    logger.setLevel(os.environ.get("LOG_LEVEL", "INFO"))
    logger.propagate = False
    logger._stormscope_configured = True  # type: ignore[attr-defined]
    return logger


F = TypeVar("F", bound=Callable[..., Any])


def retry(
    *,
    attempts: int = 4,
    base_delay: float = 1.0,
    max_delay: float = 30.0,
    exceptions: tuple[type[BaseException], ...] = (Exception,),
    log: logging.Logger | None = None,
) -> Callable[[F], F]:
    """Exponential backoff retry. Raises the last exception if every attempt fails."""

    def decorator(fn: F) -> F:
        @wraps(fn)
        def wrapper(*args: Any, **kwargs: Any) -> Any:
            delay = base_delay
            last_exc: BaseException | None = None
            for attempt in range(1, attempts + 1):
                try:
                    return fn(*args, **kwargs)
                except exceptions as exc:  # noqa: BLE001
                    last_exc = exc
                    if attempt == attempts:
                        break
                    if log is not None:
                        log.warning(
                            "retry",
                            extra={
                                "fn": fn.__name__,
                                "attempt": attempt,
                                "max_attempts": attempts,
                                "delay": delay,
                                "err": str(exc),
                            },
                        )
                    time.sleep(delay)
                    delay = min(delay * 2, max_delay)
            assert last_exc is not None
            raise last_exc
        return wrapper  # type: ignore[return-value]
    return decorator
