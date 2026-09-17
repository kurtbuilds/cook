# SSH concurrency

Cook reuses one authenticated SSH connection per host. Independent units start
concurrently, without a configured job limit. Commands and SFTP transfers share
an adaptive Tokio semaphore for session admission.

On the first `Session open refused by peer` error for a connection, Cook emits a
warning to stderr, even without verbose logging. It removes unused semaphore
capacity and retires the refused opening's slot. Further refusals reduce the
limit again. Pending openings wait for permits released by completed commands
or closed SFTP sessions. The learned limit lasts for that connection.

Only session opens are retried. Commands that have started are never replayed;
command failures and unrelated connection or permission errors propagate.
Retries pause briefly to allow server-side session cleanup. Three refusals
while already limited to one session propagate the error, so a server that
refuses every session cannot cause an endless retry loop.

OpenSSH's multiplex protocol does not preserve the specific server-side reason
for a channel-open refusal. `MaxSessions` exhaustion produces the same reply as
other channel-open refusals. The warning therefore says *possibly MaxSessions*.
See [OpenSSH multiplex session confirmation](https://github.com/openssh/openssh-portable/blob/master/mux.c).

Permits cover command execution and the full SFTP lifetime. Built-in rules close
SFTP before opening subsequent commands. There is no `--jobs`, `-j`, or
`COOK_JOBS` setting.

Run the read-only integration test with `just test-ssh user@host`. It runs 32
concurrent command/SFTP operations and expects the server's session limit to be
below 32. It runs `sleep` and `true` and opens SFTP without writing remote files.
