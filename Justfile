set next

check:
    cargo check
# --workspace because default-members is just `cli`, which has no tests.
test:
    cargo test --workspace
install:
    cargo install --path cli

fmt:
    cargo fmt -p cook -p cook_cli -p cook_agent

# Read-only integration test: command sessions and SFTP, no remote file writes.
test-ssh host:
    COOK_TEST_SSH_HOST="$host" cargo test -p cook --features ssh live_ssh_session_pressure -- --ignored --nocapture
