# Cook

Cook is a declarative tool for managing infrastructure. It is alternative to ansible, saltstack, terraform, pulumi, and others.

Let's confirm we can connect to a remote host.

```bash
cook ssh -H user@host echo hello world
```

Let's add the host.

```bash
echo "host user@host" > Cookfile
```

Now let's add a user.

```bash
echo "user michaeljackson" >> Cookfile
```

Now let's apply the changes.

```bash
cook up
```

You've now configured the server!

Cook runs independent units concurrently over one SSH connection. If the server
refuses new sessions, Cook warns once and automatically queues session openings.
See [SSH concurrency](docs/ssh.md).

Here are some other common commands:

Run a rule as a one-off:

```bash
cook run package postgresql
```

## Restarting services

A running service restarts when its unit file changes, or when a unit named
in its `restart_on` applies a change in the same run:

```kdl
cp config/vector.yaml /etc/vector/vector.yaml
service vector restart_on="file:/etc/vector/vector.yaml"
```

`restart_on` takes space-separated unit references, like `requires`, and
orders those units first. The unit file argument is optional. Without it, cook
does not write or enable the unit (a package already did) and only manages its
restarts. A stopped service is not restarted.


## Installing the daemon

By default, cook will detect the operating system
