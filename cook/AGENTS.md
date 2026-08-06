cook `Rule`s are declarations of your infrastructure.


When you add a new rule type

- Its `Rule::kind` is the resource kind, not the config keyword that produced it.
  Rules that describe the same resource share a kind (`file` and `cp` both yield
  `file`), because `kind` plus a rule's `identifier` is what makes a unit unique.
