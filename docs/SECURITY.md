# Reporting a vulnerability

Please do not open a public issue for a security problem.

Report it privately through GitHub's **Report a vulnerability** flow on this repository's Security
tab. Include what you did, what happened, and what you expected — a proof of concept helps but is
not required to file.

## What counts

Anything that would let somebody read vault contents without the master password, weaken the
encryption or the key derivation, escape the interface's content policy, or reach the API without a
valid session token.

## What does not

The limits in [THREAT_MODEL.md](THREAT_MODEL.md) are deliberate and documented: malware running as
the user, a compromised browser, screenshots, and the absence of any password recovery. A report
that Keyhold cannot survive one of those describes the design rather than a flaw in it — though an
argument that one of those limits is drawn in the wrong place is genuinely welcome, as an issue.

## Expectations

This is a personal open-source project, maintained without a schedule or a support commitment. You
will get a human answer, not an SLA.
