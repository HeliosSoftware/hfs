#!/usr/bin/env bash
# Collect hardware / software facts for a measurement report. Linux/WSL bash only.
# Usage: bin/hwinfo.sh [mongo-container-name]   (prints Markdown; redirect to a file)
# Run it INSIDE the WSL2 distro. Windows-side facts that WSL cannot see (Windows build, .wslconfig) are read
# through interop (cmd.exe / powershell.exe / wslpath) when available; otherwise it says what to paste by hand.
C=${1:-${MONGO_CONTAINER:-hfs-bench-mongo}}
sec() { printf '\n## %s\n\n' "$1"; }
blk() { printf '```\n'; cat; printf '```\n'; }
echo "# Hardware and software info"; echo; echo "Collected: $(date -u +%FT%TZ) on host $(hostname)"
sec "CPU"; { lscpu | grep -E 'Model name|^CPU\(s\)|Thread|Core|Socket|MHz|L3'; echo "nproc=$(nproc)"; } 2>&1 | blk
sec "Memory (as seen by this Linux VM)"; { grep -E 'MemTotal|MemAvailable|SwapTotal' /proc/meminfo; } | blk
sec "Disk (filesystem holding this kit and the Docker data)"
{ df -hT . 2>&1; echo; lsblk -d -o NAME,SIZE,ROTA,TYPE,MODEL 2>/dev/null; echo "(ROTA=0 means SSD/NVMe as reported by the VM; virtual disks always look like this, say the physical type by hand)"; } | blk
echo; echo "IMPORTANT: keep the kit, the repo and all data under the Linux filesystem (e.g. \$HOME), never under /mnt/c."
case "$(pwd -P)" in /mnt/*) echo "WARNING: the current directory is on a Windows drive (/mnt/...): results will be slow and not comparable." ;; esac
sec "Kernel / distro / WSL"
{ uname -a; grep PRETTY_NAME /etc/os-release; echo "WSL_DISTRO_NAME=${WSL_DISTRO_NAME:-}"; grep -qi microsoft /proc/version && echo "running under WSL: yes" || echo "running under WSL: no"; } | blk
sec "Windows side (via interop; may be empty)"
{
  if command -v cmd.exe >/dev/null 2>&1; then
    cmd.exe /c ver 2>/dev/null | tr -d '\r'
    (cd /mnt/c 2>/dev/null && cmd.exe /c "wsl.exe --version" 2>/dev/null | tr -d '\r\0')
    WU=$(cmd.exe /c "echo %USERPROFILE%" 2>/dev/null | tr -d '\r')
    if [ -n "$WU" ] && command -v wslpath >/dev/null; then
      P=$(wslpath "$WU" 2>/dev/null)/.wslconfig
      echo "--- $P"; cat "$P" 2>/dev/null || echo "(no .wslconfig: WSL defaults apply = 50% of RAM, all cores, 25% swap-ish)"
    fi
    command -v powershell.exe >/dev/null && powershell.exe -NoProfile -Command "Get-PhysicalDisk | Select FriendlyName,MediaType,Size | Format-Table -AutoSize | Out-String" 2>/dev/null | tr -d '\r'
  else
    echo "interop disabled: paste by hand: output of 'ver', 'wsl --version', the contents of %USERPROFILE%\\.wslconfig, and the physical disk model/type"
  fi
} | blk
sec "Docker"
{ docker version --format 'client {{.Client.Version}} / server {{.Server.Version}} ({{.Server.Os}})' 2>&1
  docker info --format 'Name={{.Name}} OS="{{.OperatingSystem}}" Kernel={{.KernelVersion}} NCPU={{.NCPU}} MemTotal={{.MemTotal}} CgroupVersion={{.CgroupVersion}} Driver={{.Driver}}' 2>&1
  if docker info 2>/dev/null | grep -qi 'docker desktop'; then echo "flavour: Docker Desktop (WSL integration)"; else echo "flavour: docker engine inside WSL (or unknown)"; fi; } | blk
sec "MongoDB container ($C)"
{ if docker inspect "$C" >/dev/null 2>&1; then
    docker inspect "$C" --format 'image={{.Config.Image}}
cmd={{json .Config.Cmd}}
memory={{.HostConfig.Memory}} memswap={{.HostConfig.MemorySwap}} cpus(nano)={{.HostConfig.NanoCpus}}
ulimits={{json .HostConfig.Ulimits}}
ports={{json .HostConfig.PortBindings}}
mounts={{range .Mounts}}{{.Type}}:{{.Name}}{{.Source}}->{{.Destination}} {{end}}'
    docker exec "$C" mongosh --quiet --eval 'JSON.stringify({version:db.version(), rsHosts:rs.conf().members.map(m=>m.host), txnLifetimeS:db.adminCommand({getParameter:1,transactionLifetimeLimitSeconds:1}).transactionLifetimeLimitSeconds, wtCacheBytes:Number(db.serverStatus().wiredTiger.cache["maximum bytes configured"]), oplogMB:db.getSiblingDB("local").oplog.rs.stats().maxSize/1048576})' 2>&1
  else echo "container $C not found"; fi; } | blk
sec "Toolchain"
{ rustc --version 2>&1; cargo --version 2>&1; python3 --version 2>&1; git --version; clang --version 2>&1 | head -1; ld.lld --version 2>&1 | head -1; } | blk
sec "Load right now"; cat /proc/loadavg | blk
