# Setting up AWS for Keyhold

> **Status.** Vaults, attachments and the permissions for them work today. The audit trail, shared
> links and the bucket tags described below are being built: the template already creates what they
> need, which is harmless until a Keyhold version uses it, and this page will say which version does.

This page is for whoever administers the AWS account. The person using Keyhold configures nothing
beyond picking the account, the region, the bucket and (optionally) the path when creating the vault.
Everything else, including whether the audit trail is on, comes from the account.

## Local vault or AWS vault

| Choose | When |
|---|---|
| **Local** | The secrets are yours, or a team shares them some other way. Nothing leaves the machine, it works offline, and deleting goes to a trash first. |
| **AWS Parameter Store** | A team already keeps credentials in Parameter Store, or should: access is decided by IAM, applications read the same parameters, and the account keeps the history. |

Both live side by side in the same Keyhold, and records can be copied between them. The rest of this
page is about the AWS kind. The [threat model](THREAT_MODEL.md) says what each one protects.

## What an AWS vault uses

| Piece | What for | Created by |
|---|---|---|
| IAM Identity Center | Sign-in (device login), one role per person per account | you |
| Parameter Store | One `SecureString` parameter per record; folders are the path | Keyhold, as people save |
| An S3 bucket | Attachments, and the short-lived files of a shared link | [the template](aws/keyhold.cfn.yaml) |
| An access log bucket | Every request to the attachments bucket, for investigating a shared link | the template |
| CloudWatch Logs | The audit trail: one log group per permission boundary | the template |

Keyhold never creates a bucket, a log group or a role. It probes what exists when the person signs
in, and says on screen what it found.

## Permission boundaries

Keyhold treats the **first segment of a parameter's path** as a permission boundary, because that is
how IAM policies on Parameter Store are usually written. A common layout:

| Path | Who |
|---|---|
| `/team/...` | everybody on the team |
| `/admins/...` | administrators only |

Attachments mirror the path without the leading slash (`team/db/main/ca.pem`), shared links live in
`<segment>/.keyhold/shares/`, and each segment has its own audit log group, `/keyhold/audit/<segment>`.
Parameters directly at the root of the account (`/db`) use `/keyhold/audit/#root`. So one policy per
segment covers everything Keyhold does there.

The name `.keyhold` is reserved: Keyhold refuses to create a record or a folder called that, and does
not list it as an attachment.

## Step by step

1. **Deploy the template** in each account and region that holds vaults:

   ```sh
   aws cloudformation deploy \
     --template-file docs/aws/keyhold.cfn.yaml \
     --stack-name keyhold \
     --parameter-overrides BucketName=keyhold-acme PrimarySegment=team SecondarySegment=admins
   ```

   It creates the bucket (private, SSE-KMS with a bucket key, versioned), its access log bucket, the
   lifecycle that cleans up shared links, a bucket policy (TLS only, no signature older than 12
   hours, never a CloudFront origin) and the audit log groups with their retention.

2. **Turn on the attribute for access control** in IAM Identity Center (Settings, Attributes for
   access control): key `keyhold-user`, value `${path:userName}`. It becomes a session tag, and the
   audit policy uses it to give each person a log stream only they can write. Usernames must not
   contain `:` or `*`, which log stream names refuse.

3. **Give each permission set the policies it needs**, from [`aws/policies`](aws/policies), with
   `REGION`, `ACCOUNT_ID`, `BUCKET` and `SEGMENT` replaced (one copy of the statements per segment):

   | Policy | Gives | Typical holder |
   |---|---|---|
   | [`read-only.json`](aws/policies/read-only.json) | read records and attachments in a segment | everybody who uses it |
   | [`read-write.json`](aws/policies/read-write.json) | also create, change, move and delete them | people who maintain it |
   | [`audit-write.json`](aws/policies/audit-write.json) | write one's own audit stream in that segment | everybody (see below) |
   | [`audit-read.json`](aws/policies/audit-read.json) | read the whole audit trail of the segment | everybody, or only administrators |
   | [`share.json`](aws/policies/share.json) | create and revoke shared links in the segment | people who may share |

   The `kms:*` statements only matter when a parameter or the bucket uses a **customer managed** key
   whose key policy defers to IAM. The AWS managed keys (`aws/ssm`, `aws/s3`) already allow use
   through the service for principals the service allows.

4. **Create the vault in Keyhold**: Ajustes, Contas AWS, sign in, add the account and role, then
   Novo cofre, AWS Parameter Store, with the bucket from the template's output and, for a role
   limited to one segment, the path (`/team`).

## The audit trail

Keyhold writes one event for each of: session opened, record viewed, field revealed, field copied,
record created or edited (field names, never values), moved, deleted, attachment downloaded, vault
analysed, record shared, and a shared link opened or revealed. An event names the record and the
field, never a value.

- **Who and when come from AWS, not from the event.** Each person writes only to the stream named
  after them (the policy pins it with `${aws:PrincipalTag/keyhold-user}`), and Keyhold shows the
  stream's name as the person and the time CloudWatch received the event as the time. A person with
  the CLI can write junk into their own stream; they cannot write into anybody else's, and cannot
  backdate what the screen shows.
- **Write-only for users.** Nobody but an administrator can delete a log stream or change the
  retention. For a trail even administrators cannot touch, add a subscription filter that copies
  each log group into a separate log archive account.
- **Reading is per log group.** CloudWatch Logs cannot grant reading one stream or one record.
  Whoever can read a segment's log group reads every event in it, including record titles and the
  e-mail addresses things were shared with. That is the same group of people who can already read
  the segment's records.
- **What it does not see.** Somebody using the console or the CLI does not go through Keyhold. For
  that, CloudTrail records every `GetParameter` with the person and their IP address (90 days in the
  event history, free). Keyhold reads a secret from AWS when a record is opened and again when a
  secret field is revealed or copied, so each of those is its own `GetParameter` in CloudTrail too.
  Attachment downloads only appear in CloudTrail with S3 data events, which you turn on separately.

### When the trail is not available

On sign-in and on "Renovar", Keyhold writes a "session opened" event to probe. On screen:

| Probe answer | Means | Keyhold |
|---|---|---|
| accepted | the trail is on | sensitive actions wait until AWS accepts their event |
| `AccessDenied` | no `audit-write` policy, or the Identity Center attribute is off | shows "sem trilha na AWS" with the reason, and works |
| `ResourceNotFound` | the log group does not exist | the same |
| network error, timeout, anything else | unknown | blocks revealing, copying, sharing, moving, deleting and downloading until it can write |

To make the trail mandatory, tag the bucket `keyhold:audit=required`: then "sem trilha" blocks
sensitive actions instead of only saying so.

## Shared links

A record can be shared with somebody who has neither Keyhold nor AWS access, as a link to a page in
the bucket that opens in any browser.

- **The bucket stays private.** The link is a presigned URL: one method, one object, until it
  expires. Changing anything in it breaks the signature, and nothing can be listed with it. The
  person who receives it reaches that share and nothing else.
- **AWS only ever holds ciphertext.** The key is in the link's fragment (the part after `#`, which
  browsers never send), and an optional password, sent by another channel, is needed too.
- **The link lasts at most as long as the sharer's AWS session.** A presigned URL dies with the
  temporary credentials that signed it, so the permission set's session duration (1 to 12 hours) is
  the longest a link can live.
- **"First access" is enforced by the page, not by S3.** The page deletes the share after it shows
  it. Somebody who downloads the encrypted file directly does not trigger that, which is why first
  access always needs a password. The access log bucket records every download, with IP address, for
  an administrator to investigate.
- **Whoever can read an attachment can produce a link to it**, with Keyhold or with the CLI; a
  presigned URL only passes on access its signer already has. The bucket policy limits how old a
  signature may be (12 hours) rather than denying presigned URLs, because denying them also takes
  the S3 console away from everybody.

Tag the bucket `keyhold:share=off` to turn sharing off for every vault on it.

## Tags Keyhold reads on the bucket

| Tag | Effect |
|---|---|
| `keyhold:audit=required` | no audit trail means no sensitive actions |
| `keyhold:audit-log-group` | prefix to use instead of `/keyhold/audit` |
| `keyhold:share=off` | no shared links from vaults on this bucket |
| `keyhold:share-prefix` | where shares of root-level parameters go (otherwise they cannot be shared) |

Reading them needs `s3:GetBucketTagging`, which the example policies include. When Keyhold cannot
read them it says so on screen and uses the defaults.

## Things that break it

- **SCPs that limit presigned URLs** (by source IP, VPC endpoint or `s3:signatureAge` shorter than
  the link) make shared links fail for the recipient.
- **A session duration of 1 hour** (the Identity Center default) makes every link last at most one
  hour.
- **`ssm:DescribeParameters`** is optional: it cannot be limited to a path, so many accounts leave it
  out, and Keyhold then reads titles and folder names from each parameter's history.

## Cost

For a team, effectively nothing: Parameter Store standard parameters are free, the audit trail is a
few megabytes a month (inside the CloudWatch Logs free tier), and S3 holds attachments and a few
short-lived files. Check the current prices for your region.
