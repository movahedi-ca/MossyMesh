$gh = "C:\Program Files\GitHub CLI\gh.exe"

$discussions = @(
    @{ title="Has anyone tested LoRa range in a dense urban environment?"; body="I'm getting around 2km line of sight, but in the city it drops to 300m." },
    @{ title="Best Android device for Wi-Fi Direct Group Owner?"; body="Looking for devices with large batteries to act as persistent mesh nodes." },
    @{ title="Are we moving to DAG-CBOR entirely?"; body="I saw some talk about dropping JSON serialization in the consensus layer." },
    @{ title="Issues compiling WAMR on Apple Silicon?"; body="I'm getting weird pointer alignment errors when building the sandbox on M3 Macs." },
    @{ title="Captive Portal HTML customization options"; body="Is there a way to theme the offline captive portal without rebuilding the nginx docker image?" },
    @{ title="How to manually sync Kademlia routing tables"; body="My node got isolated after moving between cities. Is there a command to force a ping?" },
    @{ title="Offline TWAMM: What happens if mid-price diverges significantly?"; body="If a node is offline for a month, does the 2% spread cap just block all trades?" },
    @{ title="Feature Request: Support for Chinese chess (Xiangqi)"; body="The bitboard engine seems flexible enough. Anyone interested in porting the rules?" },
    @{ title="Running the node headless on a Raspberry Pi Zero W?"; body="Does the 512MB RAM hold up against the trie database and the engine WASM?" },
    @{ title="Is WebBluetooth relay actually viable on iOS?"; body="I read that Apple severely restricts WebBluetooth in Safari. How does the frontend handle this?" }
)

$replies = @(
    "That's a great question, I was wondering the same thing.",
    "I've tested a similar setup and it works surprisingly well if configured correctly.",
    "Let's bring this up in the next community call to decide on a standard approach."
)

foreach ($disc in $discussions) {
    Write-Host "Creating discussion: $($disc.title)"
    $url = & $gh discussion create --title $disc.title --body $disc.body --category "General"
    
    if ($url) {
        $url = $url.Trim()
        Write-Host "Created at $url. Adding replies..."
        
        $count = 1
        foreach ($reply in $replies) {
            $replyBody = "Reply $count/3: $reply"
            & $gh discussion comment $url --body $replyBody
            $count++
            Start-Sleep -Seconds 1
        }
    }
}
Write-Host "Finished creating 10 discussions with 3 replies each."
