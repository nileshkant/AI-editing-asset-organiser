param([Parameter(Mandatory=$true)][string]$Artifact, [Parameter(Mandatory=$true)][string]$ExpectedThumbprint)
$ErrorActionPreference = 'Stop'
if ($ExpectedThumbprint -notmatch '^[A-Fa-f0-9]{40}$') { throw 'Expected publisher certificate thumbprint required' }
$signature = Get-AuthenticodeSignature -LiteralPath $Artifact
if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Thumbprint -ne $ExpectedThumbprint) { throw 'Invalid signature or unexpected publisher' }
