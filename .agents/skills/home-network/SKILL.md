---
name: home-network
description: >-
  Work on FRANK, MACINDOZE, devbox or Mitch's MacBook Pro over SSH, and share
  files between them through the NAS. Use for remote debugging, file searches,
  command execution, NAS file sharing and collecting iRacing data from the
  home network.
---

# Home network

Read the [access guide](../../../docs/home-network.md) for the machine map,
launcher setup, credentials, NAS roots and data-source rules. Run the repository's launcher
from its checkout; use `--help` for the available commands.

1. **Target.** Resolve the requested machine through the guide. For iRacing work,
   apply its source-of-truth rule before interpreting local copies. Run
   `<launcher> <machine> check`; proceed when the reported identity is the intended
   machine and SSH/SFTP both succeed. If it fails, report that machine and the
   connection, authentication or host-key error; keep the password out of output.
2. **Inspect.** Choose the branch that answers the task:
   - Debug a remote failure with `exec`: inspect the relevant processes, services,
     configuration and bounded log excerpts in the destination's native shell.
     Publish larger logs to the NAS for local analysis.
   - Search remote files with `ls`, then a directory-scoped `exec` search. Use
     PowerShell on Windows targets, the Mac's shell on the MacBook and bash on
     devbox. Quote the entire remote command as one local argument. A successful
     search identifies exact remote paths; an empty search identifies the
     directory and filter tried.
   - Share files through the NAS. The producer copies them into the NAS
     `corpus/.staging` directory and renames them into place, following the
     guide's publish steps. The consumer reads them from its own NAS root, which
     `<launcher> nas` prints. Sharing is complete when the expected files and
     their source machine/path are established, with a collection time for
     changing data.
   - For other remote work, use `exec` within the user's requested scope. On
     FRANK, `exec` is for inspection, publishing and administration. Run a build
     or other job on FRANK or any other remote machine through the guide's job
     runner. Interactive `shell` is available when a human needs a terminal.
3. **Verify.** When the task includes a fix, apply it to the intended remote
   machine, then rerun the failing operation there. Report the target, observed
   result and remaining limitation. For sharing-only work, verify the files on
   the NAS before drawing conclusions from them.

Use the existing `HOME_NETWORK_PW` environment variable. When setup or the
credential is missing, follow the guide's preparation steps. Retain host-key
verification and existing filesystem permissions. A remote session carries the
same task scope as local work; the skill itself adds no authority to restart,
delete, deploy or change unrelated resources.
