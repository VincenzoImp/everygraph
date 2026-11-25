# Security policy

## Supported versions

None yet. Everygraph has had no release; the Rust workspace is a skeleton, and there is
nothing to install. This section will list the supported versions from the first release
(0.1) on.

## Reporting a vulnerability

Do not report a vulnerability in a public issue, pull request or discussion.

- **Once the repository is public**, report it through GitHub's private vulnerability
  reporting: the repository's **Security** tab, then **Report a vulnerability**. The
  report and the conversation about it stay visible only to you and the maintainers until
  an advisory is published.
- **While the repository is private** (until 0.1), everyone who can read it is a
  collaborator invited by the owner. GitHub offers private vulnerability reporting for
  public repositories only, so write to the repository owner at vincenzo@imperati.dev,
  rather than in an issue.

Please include what is affected (crate, command or file, and the commit), how to
reproduce it, and what an attacker gains.

What happens next: the project has one maintainer today (see
[GOVERNANCE.md](GOVERNANCE.md)), so there is no security team and no promised response
time, and no vulnerability has been handled yet. The intended handling uses GitHub's
repository security advisories: a draft advisory, the fix prepared in its temporary
private fork, and the advisory published with the release that contains the fix,
crediting the reporter unless they decline.

## Scope

In scope: code under `crates/` and the software built from it.

- Memory unsafety, or undefined behaviour reachable from safe use of the public API.
- Anything a crafted input file can do beyond being rejected with an error: reading or
  writing outside the files the user named, running code, exhausting memory past the
  budget the user set, or hanging. This covers every format the engine reads — edge
  lists, Parquet, the `.eg` container, map files — because users open files they did not
  make.
- Script execution or data exfiltration through a map exported as a static file and
  opened in a browser.
- A local server that listens on anything other than the loopback interface without
  the user asking for it.
- The supply chain of this repository: CI workflows, dependency policy (`deny.toml`), and
  release artefacts once there are any.

Out of scope:

- The prototypes under `lab/` and the mockups under `design/`. They are research and
  design material, not software anyone is meant to run on untrusted input. A flaw there
  that also exists in `crates/` is in scope there.
- Wrong results, crashes on valid input and slowness. These are bugs: report them as
  ordinary issues.
- Vulnerabilities in a dependency that do not affect how Everygraph uses it: report
  them to the dependency. `cargo deny check` reads the RustSec advisory database for
  known ones.
