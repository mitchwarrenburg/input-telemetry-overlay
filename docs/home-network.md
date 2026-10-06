# Home network access

Use this repository's `scripts/home-network.ps1` on Windows or
`bash scripts/home-network.sh` on macOS to reach the other home computers by SSH
or SFTP. Run the examples from the repository root. The same commands and aliases
work in every participating repository.

## Choose the source

| Machine | Aliases | SSH account and address |
| --- | --- | --- |
| FRANK — primary machine | `win` | `mitch@192.168.1.247` |
| MACINDOZE | `imac` | `mitch@192.168.1.243` |
| Mitch's MacBook Pro | `macbook`, `mitch-mac`, `mitch-mac.local` | `mitchwarrenburg@mitch-mac.local` |

Machine names and aliases are case-insensitive. Quote names containing spaces;
the MacBook name accepts either a straight or curly apostrophe.

**iRacing runs and produces data only on FRANK.** For tasks needing iRacing
captures, telemetry, logs or other simulator output, locate the required files on
FRANK and collect them through SSH, SFTP or SMB. Local copies on another machine
are snapshots; record their source path and collection time when freshness matters.
Discover the actual directories for each task rather than assuming another
machine's checkout contains the current data.

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
PowerShell on FRANK and MACINDOZE, the account's configured shell on the MacBook.
Quote for the local shell first; single quotes preserve a remote PowerShell `$`
expression in both PowerShell and Bash. `check` verifies authentication and a
remote command. Use `ls` to choose an actual source path before transferring it.

File transfer forms, with placeholder paths to replace after discovery:

```powershell
.\scripts\home-network.ps1 FRANK get 'D:/actual/path/session.ibt' './session.ibt'
.\scripts\home-network.ps1 FRANK get 'D:/actual/capture-directory' './capture-copy' --recursive
.\scripts\home-network.ps1 imac put './result.json' 'D:/actual/output/result.json'
```

```sh
bash scripts/home-network.sh FRANK get 'D:/actual/path/session.ibt' './session.ibt'
bash scripts/home-network.sh macbook put './result.json' '/Users/mitchwarrenburg/actual/output/result.json'
```

Transfers take exact destination paths and refuse to overwrite existing files.
Choose a new destination when collecting another snapshot. Directory transfers
require `--recursive` and reject symlinks. Use forward slashes for remote Windows
paths (`C:/...` or `/C:/...`) and absolute paths for the MacBook.

Agents should use `check`, `ls`, `exec` and `get` for bounded tasks, then verify
the retrieved files have the expected contents and provenance. For an interactive
terminal or file-transfer session, use `<launcher> FRANK shell` or
`<launcher> FRANK sftp`.

## FRANK's SMB shares

FRANK shares its C, D and E drives. C is read-only over SMB; D and E are writable
as `mitch`, subject to existing filesystem permissions.

| Drive | Windows Explorer | macOS Finder → Go → Connect to Server |
| --- | --- | --- |
| C — read-only | `\\FRANK\C` | `smb://FRANK/C` |
| D | `\\FRANK\D` | `smb://FRANK/D` |
| E | `\\FRANK\E` | `smb://FRANK/E` |

Use `<launcher> FRANK smb` to display the share addresses. If FRANK's name does
not resolve, replace `FRANK` with `192.168.1.247`. Authenticate as `mitch` using
the shared password when the operating system asks. The SMB client handles its
own login; the script's environment variable supplies SSH/SFTP authentication.
C's read-only share setting applies to SMB; SSH and SFTP follow NTFS permissions.

## Connection checks

The first SSH connection accepts a new host key and displays its fingerprint,
then saves it in `~/.ssh/known_hosts`, shared with OpenSSH and the other repositories. A changed key fails;
verify the destination and its new fingerprint before replacing an old entry.
Keep host-key verification enabled.

All three addresses require home-network reachability. `mitch-mac.local` uses
local name resolution, and sleeping machines may be unreachable. If DHCP changes
a Windows address, update the helper's host mapping and this guide in the
participating repositories. Use `info` to inspect the selected destination and
`check` to distinguish connection or authentication failures from file-path errors.
