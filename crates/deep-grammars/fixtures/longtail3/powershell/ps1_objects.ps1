using namespace System.Text

enum Severity {
    Low
    Medium
    High
}

class Deployment {
    [string]$Name
    [Severity]$Level = [Severity]::Low
    hidden [int]$Retries = 3

    static [Deployment] Create([string]$name) {
        $made = [Deployment]::new()
        $made.Name = $name
        return $made
    }

    [string] Describe() {
        return "$($this.Name) at $($this.Level) with $($this.Retries) retries"
    }

    [void] Apply([string]$target) {
        Write-Verbose "applying $($this.Name) to $target"
        $this.Retries--
    }
}

function global:Deploy-Thing {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,
        [switch]$Force
    )

    $deployment = [Deployment]::Create($Name)
    if ($Force -or $deployment.Retries -gt 0) {
        $deployment.Apply("production")
    }

    return $deployment.Describe()
}

function Get-Item-Or-Null {
    param([string]$Path)

    try {
        return Get-Item -Path $Path -ErrorAction Stop
    } catch [System.IO.FileNotFoundException] {
        Write-Error "not found: $Path"
        return $null
    } finally {
        Write-Verbose "looked for $Path"
    }
}
