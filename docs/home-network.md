# Home network access

Use this repository's `scripts/home-network.ps1` on Windows or
`bash scripts/home-network.sh` on macOS or devbox to run commands on the other
home computers over SSH. Files move between machines through the NAS. See
[Share files through the NAS](#share-files-through-the-nas). Run the examples
from the repository root. The same commands and aliases work in every
participating repository.

## Ask an agent

The repo-local [home-network skill](../.agents/skills/home-network/SKILL.md) is
available to Codex as `$home-network` and Claude Code as `/home-network`. It also
applies automatically to matching remote-work requests. For example:

- "Use home-network to debug the capture collector on FRANK."
- "Search macbook's project directory for the latest error log."
- "Publish the required iRacing session files from FRANK to the NAS and analyze them here."

Codex reads the canonical skill under `.agents/skills/home-network`; Claude's
`.claude/skills/home-network` entry points to the same instructions. Start a new
agent session after pulling these files if the skill is absent from its catalog.

## Choose the source

| Machine | Aliases | SSH account and address |
| --- | --- | --- |
| FRANK, primary machine | `win` | `mitch@192.168.1.247` |
| MACINDOZE | `imac` | `mitch@192.168.1.249` |
| Mitch's MacBook Pro | `macbook`, `mitch-mac`, `mitch-mac.local`, `Mac.attlocal.net` | `mitchwarrenburg@mitch-mac.local` |
| devbox, the Linux VM on the NAS | `nas-vm` | `mitch@192.168.1.20` |

The NAS is `truenas.local` (`192.168.1.253`). It has no SSH login. Reach its
files through the share and its settings through its web UI. devbox runs the
Garage 61 collector.

Machine names and aliases are case-insensitive. Quote names containing spaces;
the MacBook name accepts either a straight or curly apostrophe.

**iRacing runs and produces data only on FRANK.** A scheduled task there
publishes `.ibt` telemetry to `corpus/telemetry` and the capture tree
`D:\PitwireData\iracing-captures` to `corpus/iracing-captures` on the NAS every
30 minutes while the simulator is not running. For other captures, logs or simulator output,
locate the files on FRANK over SSH, publish them to the NAS, and read them from
the NAS. Copies on the NAS are snapshots; record their source path and
collection time when freshness matters. Discover the actual directories for each
task rather than assuming another machine's checkout contains the current data.

FRANK is reserved for iRacing. The `mitch` login there, the only one this
helper uses, is for inspection, publishing and administration. Builds and other
jobs enter as `pitwire-job`, which runs only while Mitch holds a lease open and
the simulator, `iRacingSim64DX11`, is not running. Start them with the
[job runner](#run-a-job-on-another-machine). A refusal from that login, exit
code 77, means FRANK is not accepting jobs, so run the job elsewhere. The login's key is
`~/.ssh/pitwire_job` on the MacBook. Another machine needs its own key pair,
with the public key added to `C:\ProgramData\Pitwire\job_authorized_keys` on
FRANK by Mitch.

## Prepare once per machine

Install Python 3.9 or newer, then run the appropriate launcher:

```powershell
.\scripts\home-network.ps1 setup
```

```sh
bash scripts/home-network.sh setup
```

`setup` creates an isolated environment under your user account and installs
Paramiko there. Other repositories reuse that environment; application
dependencies remain separate. `hosts` lists the configured machines without
connecting, and `--help` describes the full command interface.

Supply the shared login password through `HOME_NETWORK_PW` in the terminal that
will run the script. Substitute the actual password for the placeholder:

```powershell
$env:HOME_NETWORK_PW = '<Windows password>'
```

```sh
export HOME_NETWORK_PW='<Windows password>'
```

Keep the value in the local environment. Git, example files, command output and
task reports must remain free of the password. A saved Windows user environment
variable is inherited by newly opened terminals; existing terminals need their
own session value.

## Connect and collect

The syntax is `<launcher> <machine> <action> [arguments]`. For example, on Windows:

```powershell
.\scripts\home-network.ps1 hosts
.\scripts\home-network.ps1 FRANK info
.\scripts\home-network.ps1 win check
.\scripts\home-network.ps1 FRANK ls C:/Users/mitch
.\scripts\home-network.ps1 FRANK ls D:/
.\scripts\home-network.ps1 FRANK exec 'Get-ChildItem -LiteralPath C:/Users/mitch -Directory | Select-Object FullName'
.\scripts\home-network.ps1 macbook exec 'uname -a'
```

On macOS, use the same actions with the Bash launcher:

```sh
bash scripts/home-network.sh imac check
bash scripts/home-network.sh FRANK ls D:/
bash scripts/home-network.sh FRANK exec 'Get-ChildItem -LiteralPath C:/Users/mitch -Directory | Select-Object FullName'
bash scripts/home-network.sh "Mitch's MacBook Pro" info
```

`exec` takes one quoted command string interpreted by the destination's shell:
PowerShell on FRANK and MACINDOZE, the account's configured shell on the MacBook
and devbox.
Quote for the local shell first; single quotes preserve a remote PowerShell `$`
expression in both PowerShell and Bash. `check` verifies authentication and a
remote command. Use `ls` to choose an actual source path before publishing it.
For Unicode output from a Windows `exec` command, start the remote command with
`[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false);` so PowerShell
emits UTF-8 over SSH.

Agents should use `check`, `ls` and `exec` for bounded tasks. For an interactive
terminal, use `<launcher> FRANK shell`.

## Share files through the NAS

Every file that moves between machines goes through the NAS share. The producer
publishes and the consumer reads. `<launcher> nas` prints this computer's root:

| Computer | Root |
| --- | --- |
| Windows | `\\truenas\files\pitwire` |
| macOS | `<files mount>/pitwire`, usually `/Volumes/files/pitwire` |
| devbox | `/pitwire`, which holds `corpus` and `git` only |

On macOS, mount the share first: Finder, Go, Connect to Server,
`smb://truenas.local/files`, as `mitch`. A Windows SSH session logged in as
`mitch` reaches the UNC path directly, because the NAS has a `mitch` account
with the same password. A `pitwire-job` session on FRANK gets the share mapped
under its own NAS account for the job's lifetime. That account writes `runs` and
reads `git` and `corpus`. If `truenas` does not resolve, use `192.168.1.253`.

| Directory | Holds |
| --- | --- |
| `corpus/telemetry` | FRANK's `.ibt` files |
| `corpus/iracing-captures` | FRANK's capture tree. Empty on 2026-10-12, because the earlier corpus was deleted with FRANK's E: drive |
| `corpus/garage61/<export-id>` | Garage 61 database exports |
| `corpus/.staging/<machine>-<utc>-<name>` | publications still copying |
| `git/<repo>.git` | bare mirrors |
| `runs/<repo>/<run-id>` | one job's `job.json`, `output.log` and `receipt.json`. devbox does not mount it |
| `tools/pitwire_run.py` | the job runner |

FRANK's task keeps `corpus/telemetry` and `corpus/iracing-captures` current in
place. Everything else is published by hand. To publish, copy closed files
into `corpus/.staging/<machine>-<utc>-<name>/`, then rename that directory to
its final path under `corpus`. Both paths are in
one dataset, so the rename is atomic. Give every publication a new final path.
Consumers read only directories outside `.staging`, and copy inputs to local
disk before heavy work. For example, from macOS, publish a directory that sits
on FRANK:

```sh
bash scripts/home-network.sh FRANK exec 'if (Test-Path -LiteralPath \\truenas\files\pitwire\corpus\captures-20261012) { exit 1 }; robocopy D:\actual\capture-directory \\truenas\files\pitwire\corpus\.staging\frank-20261012T0510Z-captures /E /R:2 /W:5 /NP /NFL /NDL; if ($LASTEXITCODE -ge 8) { exit $LASTEXITCODE }; Move-Item -LiteralPath \\truenas\files\pitwire\corpus\.staging\frank-20261012T0510Z-captures -Destination \\truenas\files\pitwire\corpus\captures-20261012 -ErrorAction Stop'
```

The command exits nonzero when the final path already exists, when robocopy
reports 8 or higher, or when the rename fails. Windows shows `.staging` itself
as hidden, so PowerShell needs `-Force` to move or delete that directory.

Keep clones, worktrees, build targets and live databases on each machine's own
disk.

Read short diagnostics with `exec`, for example `Get-Content -Tail 50` on a log.
The helper's `--help` still lists `get`, `put` and `sftp`. Leave them unused,
because every file goes through the NAS.

## Run a job on another machine

The job runner on the NAS runs one command at an exact commit on a node and
leaves a receipt. From a repository checkout on the MacBook:

```sh
python3 "$(bash scripts/home-network.sh nas)/tools/pitwire_run.py" submit . --node frank -- python scripts/home_network.py hosts
```

Everything after `--` is one program and its arguments, run from the repository
root on the node. Shell operators such as `&&`, `|` and `>` do not reach the
node. The node resolves the program name, so Python is `python` on FRANK and
`python3` on the MacBook.

It pushes the commit, with tracked uncommitted changes, to the mirror under
`git/` and writes `runs/<repo>/<run-id>/job.json`. Untracked files are not
sent, so `git add` a new file first. The node runs the command in
a reused checkout on its own disk, with a private Cargo target, then writes
`output.log` and `receipt.json`. The runner exits with the command's status, and
the receipt records the commit, duration and test counts.

| Node | Runs on | Ready when |
| --- | --- | --- |
| `macbook` | this MacBook | always |
| `frank` | FRANK, as `pitwire-job` | a lease is open and the simulator is not running. Python 3.13 and Git are on the job's PATH |
| `macindoze` | MACINDOZE | its work disk is attached, which has not happened yet |

A receipt whose `state` is not `finished` means the command did not run. Two
exit codes have a set response:

| Exit code | Receipt `state` | Meaning | Do |
| --- | --- | --- | --- |
| 77 | `not_started` | FRANK is not accepting jobs | Resubmit on another node |
| 75 | `refused` | The node is busy with another run | Resubmit when that run finishes, or on another node |

## Connection checks

The first SSH connection accepts a new host key and displays its fingerprint,
then saves it in `~/.ssh/known_hosts`, shared with OpenSSH and the other repositories. A changed key fails;
verify the destination and its new fingerprint before replacing an old entry.
Keep host-key verification enabled.

All addresses require home-network reachability. devbox has a static address. `mitch-mac.local` uses
local name resolution, and sleeping machines may be unreachable. If DHCP changes
a Windows address, update the helper's host mapping and this guide in the
participating repositories. Use `info` to inspect the selected destination and
`check` to distinguish connection or authentication failures from file-path errors.
