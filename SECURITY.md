# Security

## What mulu does and does not tell you

mulu reports findings about a finite model it builds from a contract. Under
the default configuration **every finding is a claim about that model**, not
about the contract, and not about deployed bytecode. `obligations.json` in each
analysis lists what would have to be proved for a claim to reach further, and
none of it is proved today.

Read a `never-fails` as "the model says this guard's failing branch is
unreachable". It is not a licence to delete the guard: `never-fails` is not
`removal-equivalent`, which would also have to account for gas and side
effects.

An analysis that reports anything as `unsupported` or `partial` is incomplete,
and exits non-zero for that reason. Do not read a clean-looking report from an
incomplete run as an absence of problems.

## Reporting a problem in mulu itself

The reports we most want are ones where the tool said something stronger than
it should:

- a finding reported at a layer its obligations do not reach
- a behaviour of the contract that the generated model does not have
- a construct the analysis skipped without recording it as unsupported
- a certificate that checks but does not support the claim attached to it

Open an issue with the contract, the specification, the command, and the
output. A minimal contract is more useful than a real one.

For anything you would rather not discuss in public, contact the maintainers
privately through the repository's security advisory form.

## Scope

mulu compiles the Solidity you give it by running `solc`, and replays
counterexamples on an in-process EVM. It does not deploy anything, does not
open network connections, and does not modify the sources it reads.
