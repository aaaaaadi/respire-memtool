# Temporary Actions artifact cleanup

Only FUTURE producer runs that emit `cleanup-ready-v1` opt in. Existing artifacts
have no marker and are never swept retroactively. The receipt records immutable
artifact IDs, names, digests, source SHA, run attempt and delivery metadata. It is
created after the workflow's existing consumers, not merely after compilation.

The default-branch cleanup script checks the entire successful source run, every
job, and later repository workflows before every DELETE. Payloads are deleted
before the receipt. Each ID is re-fetched and deletion is verified with GET 404;
an uncertain result stops without retry. Official Release assets, tags, packages,
images and caches are outside the allowed Actions-artifact DELETE API scope.

The scheduled cleanup runs every 15 minutes, no earlier than one hour after the
last known consumer/publication/receipt. GitHub can delay or drop schedule jobs:
this is best effort, not a guaranteed one-hour expiry. An explicitly configured cross-workflow consumer
restarts the grace period; unrelated completed producers do not. All earlier cleanup runs are excluded from that clock.
A human can still start a rerun after cleanup; there is no GitHub atomic transaction
that prevents this. Expired-input reruns require a fresh full producer build.

`retention-days: 1` is GitHub's minimum supported fallback for disposable artifacts,
not a fractional one-hour setting. Failed/cancelled builds do not emit a cleanup
receipt and keep that fallback. Existing external deployment-input retention is
preserved until explicit delivery acknowledgment. Manual cleanup defaults to a
dry run; the dedicated cleanup job alone adds `actions: write`, using the existing
GITHUB_TOKEN. Other new jobs have only contents/actions read. No new secret,
global permission, installation, or external data sink is introduced. Audit IDs
and outcomes remain in job logs/summary, without another stored artifact.

## This repository's consumer contract

- `release.yml`: `release`; allowlist: `windows`, `macos`, `linux`.

All three platform artifacts are consumed by the existing release job first.
Only its completed successful public release, exact tag/source and unchanged
asset sizes/digests permit cleanup. The marker is uploaded AFTER download-all has
finished, so it cannot contaminate installer packaging.

## Validation

`python3 -m unittest discover -s .github/scripts -p 'test_cleanup_artifacts.py' -v`
checks live/failed consumers, exact hour boundary, attempt/source/ID drift,
receipt ZIP validation, deletion uncertainty, SDK delivery failure and external
acknowledgment. The separate read-only policy CI runs on PRs before activation.

References: [artifact retention minimum](https://github.com/actions/upload-artifact#retention-period),
[GitHub schedule limits](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#schedule).
