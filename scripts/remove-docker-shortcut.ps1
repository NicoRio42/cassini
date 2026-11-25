# Inserted into dist's cassini-installer.ps1 by patch-installers.sh, and
# called right after the binary is installed.
#
# Cassini used to be run with Docker, through a `cassini` function that the
# installation guide had users add to their PowerShell profile:
#
#     function cassini {
#       docker run --rm -it --name cassini -v "${PWD}:/app" nicorio42/cassini $args
#     }
#
# A function takes precedence over the executable in the PATH, so it would
# keep running the Docker image. This removes it from the profiles of Windows
# PowerShell and PowerShell 7, keeping a copy of each file it changes. A
# `cassini` function that doesn't run Docker is left alone. With
# CASSINI_NO_MODIFY_PATH=1, it only tells the user where the shortcut is.
function Remove-DockerShortcut($WarnOnly) {
  $documents = [Environment]::GetFolderPath("MyDocuments")
  if (-not $documents) {
    return
  }
  $profiles = @(
    "WindowsPowerShell\Microsoft.PowerShell_profile.ps1",
    "WindowsPowerShell\profile.ps1",
    "PowerShell\Microsoft.PowerShell_profile.ps1",
    "PowerShell\profile.ps1"
  ) | ForEach-Object { Join-Path $documents $_ }

  $found = @()
  foreach ($file in $profiles) {
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) {
      continue
    }
    try {
      $bytes = [IO.File]::ReadAllBytes($file)
      # Write the file back with the encoding it was read with.
      $reader = New-Object IO.StreamReader($file, (New-Object Text.UTF8Encoding($false)), $true)
      $text = $reader.ReadToEnd()
      $encoding = $reader.CurrentEncoding
      $reader.Close()
      if ($encoding -is [Text.UTF8Encoding]) {
        $hasBom = $bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF
        $encoding = New-Object Text.UTF8Encoding($hasBom)
      }

      $newline = if ($text.Contains("`r`n")) { "`r`n" } else { "`n" }
      $lines = $text -split "\r?\n"
      $kept = New-Object Collections.Generic.List[string]
      $block = $null
      $removed = $false
      foreach ($line in $lines) {
        # The start of a `cassini` function, with its brace on the same line
        # or the next one.
        if ($null -eq $block -and $line -match '^\s*function\s+cassini\s*(\{.*)?$') {
          $block = New-Object Collections.Generic.List[string]
          $depth = 0
          $opened = $false
        }
        if ($null -eq $block) {
          $kept.Add($line)
          continue
        }
        $block.Add($line)
        $opens = ([regex]::Matches($line, '\{')).Count
        $depth += $opens - ([regex]::Matches($line, '\}')).Count
        if ($opens -gt 0) {
          $opened = $true
        }
        if ($opened -and $depth -le 0) {
          if (($block -join "`n") -match 'docker') {
            $removed = $true
          } else {
            $kept.AddRange($block)
          }
          $block = $null
        }
      }
      # An unterminated block is not a function: keep it.
      if ($null -ne $block) {
        $kept.AddRange($block)
      }
      if (-not $removed) {
        continue
      }

      $found += $file
      if ($WarnOnly) {
        continue
      }
      Copy-Item -LiteralPath $file -Destination "$file.cassini-backup" -Force
      [IO.File]::WriteAllText($file, ($kept -join $newline), $encoding)
      Write-Information "removed the Docker cassini shortcut from $file (backup: $file.cassini-backup)"
    } catch {
      Write-Warning "could not remove the Docker cassini shortcut from ${file}: $($_.Exception.Message)"
    }
  }

  if ($found.Count -eq 0) {
    return
  }
  if ($WarnOnly) {
    Write-Warning "A cassini function that runs Docker is defined in: $($found -join ', ')"
    Write-Warning "Remove it, or the cassini command will keep running the Docker image."
  } else {
    Write-Information "Open a new terminal, so that the cassini command runs the installed executable."
  }
}
