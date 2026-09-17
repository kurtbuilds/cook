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


## Installing the daemon

By default, cook will detect the operating system
