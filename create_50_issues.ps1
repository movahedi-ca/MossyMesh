$gh = "C:\Program Files\GitHub CLI\gh.exe"
$issues = Get-Content .\50_issues.json | ConvertFrom-Json

foreach ($issue in $issues) {
    Write-Host "Creating issue: $($issue.title)"
    & $gh issue create --title $issue.title --body $issue.body --label $issue.label
    Start-Sleep -Seconds 1
}
Write-Host "Finished creating 50 issues."
