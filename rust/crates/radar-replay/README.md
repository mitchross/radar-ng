# radar-replay

Operator-only MRMS case imports through `radar replay check` and `radar replay import`.
See the [operator guide](../../../docs/develop/replay-importer.md) for revision contracts, quotas and recovery.

Reuse live conversions, PrecipFlag classification and tiler publication without invoking live ingest activities.
Render each frame on a plain thread. Keep staging and prior revisions under the shared replay quota.
Tile completion does not imply public readiness. Historical warnings and read-only publication remain separate work.
