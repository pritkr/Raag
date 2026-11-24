# Security Policy

## Reporting a Vulnerability

If you discover a security or privacy vulnerability in Resonance — especially involving:

- Credential, cookie (`SAPISID`, `__Secure-3PAPISID`), or `AndroidKeyStore` / OS Keychain handling
- InnerTube client cookie isolation (`ANDROID_VR` / `IOS`)
- Android WebView User-Agent / JavaScript bridge or `PlaybackForegroundService` exported components
- `LoopbackProxy` (`127.0.0.1`) binding or request-smuggling issues
- QuickJS-NG (`rquickjs`) EJS solver bundle verification (`SHA-256`)

**Please do not open a public GitHub issue.**

Instead, use GitHub's **Private Vulnerability Reporting** ("Security" tab → "Report a vulnerability") on this repository, or contact the maintainer privately with:

1. A description of the issue and affected files/components.
2. Steps or a minimal proof of concept to reproduce.
3. Potential impact and any suggested remediation.

We will acknowledge receipt and work on a fix before public disclosure.
