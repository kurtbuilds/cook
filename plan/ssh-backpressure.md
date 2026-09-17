# Adaptive SSH concurrency

## Specification

Remove `--jobs`, `-j`, and `COOK_JOBS`. Start independent units without a
configured limit over the existing shared SSH connection. On the first exact
OpenSSH session-open refusal, warn once per connection and reduce concurrency.
Queue session openings with Tokio permits. Retry only rejected opens, never
commands that have started. Count SFTP sessions for their full lifetime.
Propagate unrelated errors and permanent refusals rather than retry forever.

OpenSSH mux reports `Session open refused by peer` for channel-open failure;
it does not preserve the server's `MaxSessions` reason. Treat this as possible
session pressure, not proof of its cause.

## Work plan

- [x] Inspect OpenSSH error and SFTP lifecycle behavior.
- [x] Remove fixed limit and wrap command/SFTP opens with adaptive permits.
- [x] Test saturation, retries, failure, cancellation, and permanent refusal.
- [x] Update docs, run checks and read-only SSH validation, install Cook.

## Validation

`just check` and `just test` pass (59 tests; live test ignored by default).
`just test-ssh root@algo2` passed with 32 mixed command/SFTP operations and
exactly one warning. Live testing exposed delayed server-side session teardown;
retries now allow a short cleanup delay and stop after three serial refusals.

Installed with `just install`; installed CLI help has no jobs setting.
