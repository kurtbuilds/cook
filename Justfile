check:
    cargo check
# --workspace because default-members is just `cli`, which has no tests.
test:
    cargo test --workspace
install:
    cargo install --path cli
