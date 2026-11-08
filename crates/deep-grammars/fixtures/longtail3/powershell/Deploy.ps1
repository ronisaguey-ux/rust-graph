using namespace System

class Deployer {
    [string]$Name
    [int]$Count = 0

    [string] Describe() {
        return $this.Name + " " + $this.Count
    }

    [void] Run() {
        try {
            $this.Count = $this.Count + 1
        } catch {
            Write-Error $_
        } finally {
            Write-Host "done"
        }
    }
}

$deployer = New-Object Deployer
$deployer.Run()
