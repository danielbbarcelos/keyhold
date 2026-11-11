import { authHeaders, disown } from './session'

/**
 * Every call to the binary goes through here.
 *
 * One entry point rather than `fetch` scattered around, so the token, the failure shape and the
 * same-origin rule are decided once. `credentials: 'omit'` is explicit: keyhold authenticates with
 * a header, never a cookie, and a request that could carry ambient credentials is a request another
 * page could make on the user's behalf.
 */

export type Protection = 'password' | 'device'

export interface IdentityState {
  initialised: boolean
  protection: Protection | null
  second_factor: boolean
  recovery_confirmed: boolean
}

export interface LockPolicy {
  idle_seconds: number | null
  hidden_seconds: number | null
}

export interface LockState {
  unlocked: boolean
  locks_in: number | null
  policy: LockPolicy
}

export interface Status {
  version: string
  identity: IdentityState
  lock: LockState
  retention_days: number
  log_reads: boolean
  appearance: Appearance
  /** What "Gerar senha" starts from. */
  generator?: Recipe
  /** Widths the person dragged, in px. */
  layout?: Layout
  attemptsLeft: number
}

export interface Recipe {
  length: number
  lowercase: boolean
  uppercase: boolean
  digits: boolean
  symbols: boolean
  avoid_ambiguous: boolean
  /** Only these symbols; `null` is the usual set. */
  symbol_set: string | null
}

export const DEFAULT_RECIPE: Recipe = {
  length: 20,
  lowercase: true,
  uppercase: true,
  digits: true,
  symbols: true,
  avoid_ambiguous: false,
  symbol_set: null,
}

export interface Layout {
  tree: number
  label: number
}

/** An alert on a record. `preset` links it to a standard alert, whose current words win here. */
export interface Alert {
  preset?: string | null
  colour: string
  icon: string
  title: string
  text: string
}

export interface AlertPreset {
  id: string
  colour: string
  icon: string
  title: string
  text: string
}

export type Theme = 'grafite' | 'claro' | 'sistema'
export type Accent = 'ambar' | 'oceano' | 'floresta' | 'coral'
export type Width = 'inteira' | 'contida'

export interface Appearance {
  theme: Theme
  accent: Accent
  width: Width
}

/** What "Copiar para…" copies: a whole vault, one folder, or chosen records. */
export interface CopyScope {
  source_vault: string
  folder_id?: string | null
  record_ids?: string[] | null
  dest_vault: string
  dest_folder?: string | null
}

export interface CopyConflict {
  /** The record that already has the name, or `null` when two copied records would share it. */
  dest_id: string | null
  /** An AWS `String` parameter: never overwritten. */
  read_only: boolean
  /** `self`: the record already there is the one being copied, so overwriting is never offered. */
  reason: 'exists' | 'repeated' | 'self'
  /** The destination's version when the plan was made, for an AWS overwrite. */
  version?: number
}

export interface CopyItem {
  source_id: string
  title: string
  rel: string[]
  /** Where it lands, as the person reads it ("Bancos / Inter" or "/app/bancos/inter"). */
  path: string
  /** The parameter name in an AWS destination. */
  name: string | null
  conflict: CopyConflict | null
  attachments: number | null
  error: string | null
}

export interface CopyPlan {
  items: CopyItem[]
  source: { id: string; name: string; kind: string }
  dest: { id: string; name: string; kind: string }
  checks: { ok: boolean; problems: string[]; session_expires_at: number | null; estimated_seconds: number }
  metadata_visible: boolean
}

export interface CopyRunItem {
  source_id: string
  rel: string[]
  action: 'create' | 'overwrite' | 'rename'
  title?: string
  name?: string | null
  dest_id?: string | null
  expected_version?: number | null
}

export interface CopyRunAnswer {
  source_id: string
  dest_id: string
  /** Every attachment arrived. Only such a record may have its original deleted. */
  files_ok: boolean
  /** Sent again after a stop and already there: not written a second time. */
  already: boolean
  warning?: string
  /** The title a rename settled on (local), past names already taken. */
  title?: string
  /** The parameter name used (AWS). */
  name?: string
}

export interface AgentRequest {
  id: string
  peer: { pid: number; process: string; parent: string | null; parent_pid: number | null }
  record_id: string
  record_title: string
  field: string
  /** `get` — the value enters the conversation; `run` — it does not. */
  intent: string
  asked_at: number
}

/** How long a standing permission lasts. */
export type GrantScope =
  | { kind: 'until'; at: number }
  | { kind: 'while-running'; pid: number; since: number; process: string }
  | { kind: 'always' }

/** A permission the agent may use without asking again. */
export interface AgentGrant {
  id: string
  record_id: string
  record_title: string
  field: string
  scope: GrantScope
  created_at: string
}

/** What the approval dialog sends back: how long the yes is good for. */
export type GrantWindow =
  | { kind: 'once' }
  | { kind: 'process' }
  | { kind: 'minutes'; minutes: number }
  | { kind: 'always' }

/** One keyhold process on this machine, as `/proc` describes it. */
export interface InstanceProcess {
  pid: number
  role: 'interface' | 'agent' | 'command' | 'other'
  running_for: number
  is_this_one: boolean
  holds_the_vault: boolean
}

export interface AuditActor {
  kind: 'you' | 'agent'
  process?: string
  pid?: number
}

export interface AuditEvent {
  seq: number
  at: string
  actor: AuditActor
  action: string
  scope: string | null
  result: 'allowed' | 'denied'
  detail: Record<string, unknown>
  hash: string
}

export interface AuditIntegrity {
  intact: boolean
  checked: number
  broken_at: number | null
}

export type HealthIssue = 'compromised' | 'rotation' | 'reused' | 'weak' | 'stale'

export interface HealthFinding {
  record_id: string
  title: string
  folder_id: string | null
  issues: HealthIssue[]
  reasons: string[]
}

export interface VaultHealth {
  findings: HealthFinding[]
  examined: number
}

export class ApiError extends Error {
  constructor(
    readonly status: number,
    message: string,
    /** For AWS vaults: what kind of failure it was (`session_expired`, `access_denied`, ...). */
    readonly aws?: string,
  ) {
    super(message)
  }
}

async function call<T>(method: string, path: string, body?: unknown): Promise<T> {
  const response = await fetch(path, {
    method,
    credentials: 'omit',
    headers: {
      Accept: 'application/json',
      ...(body === undefined ? {} : { 'Content-Type': 'application/json' }),
      ...authHeaders(),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  })

  const text = await response.text()
  const payload = text ? JSON.parse(text) : {}

  if (!response.ok) {
    // Not a wrong password — those are a 401 too. This is the gate saying the token itself is not
    // one of this process's, which no amount of typing will fix.
    if (response.status === 401 && payload.session === 'invalid') {
      disown()
    }

    throw new ApiError(response.status, payload.error ?? `${path} answered ${response.status}`, payload.aws)
  }

  return payload as T
}

/** Where a vault in AWS lives. */
export interface AwsVault {
  connection_id: string
  region: string
  bucket: string
  /** Whether "Mudar o nome na AWS também" starts ticked when a title or folder name changes. */
  rename_with_title?: boolean
}

export interface AwsLogin {
  id: string
  start_url: string
  region: string
  label: string
  connected: boolean
  expires_at: number | null
  /** The session renews itself until Identity Center's own session duration ends. */
  renews?: boolean
}

export interface AwsConnection {
  id: string
  login_id: string
  account_id: string
  account_name: string
  role: string
  title: string
  note: string
  used_by: string[]
}

export interface DeviceLogin {
  user_code: string
  verification_uri_complete: string
  interval: number
  expires_at: number
}

export interface VaultSummary {
  id: string
  name: string
  description: string
  colour: string
  /** `local`, or `aws` for a vault kept in Parameter Store. */
  kind?: 'local' | 'aws'
  aws?: AwsVault | null
  /** The vault's own icon; without one it shows its initials. */
  icon?: RecordIcon | null
  agent_access: boolean
  archived: boolean
  created_at: string
  key_version: number
  key_rotated_at: string | null
}

export interface FolderNode {
  id: string
  vault_id: string
  parent_id: string | null
  name: string
  colour: string | null
  depth: number
}

export interface RecordHeader {
  id: string
  vault_id: string
  folder_id: string | null
  kind: string
  version: number
  favourite: boolean
  rotation_due_at: string | null
  compromised_at: string | null
  archived: boolean
  archived_at: string | null
  created_at: string
  updated_at: string
}

/** The fifteen kinds of field. See `fields.ts` for how each is drawn. */
export type FieldKind =
  | 'text'
  | 'multiline'
  | 'multiline_hidden'
  | 'hidden'
  | 'password'
  | 'pin'
  | 'totp'
  | 'email'
  | 'name'
  | 'phone'
  | 'url'
  | 'address'
  | 'date'
  | 'security_question'
  | 'card'

export interface Field {
  label: string
  kind: FieldKind
  value: string
  /** The other parts of a composite field: `question`; `holder`, `expiry`, `code`, `pin`. */
  extra?: Record<string, string>
  /** Shown under the title in lists. */
  list?: boolean
  /** Copied by the record's main button. */
  primary?: boolean
}

/** A lucide icon in one of the interface's colours, or a picture of the person's own. */
export interface RecordIcon {
  name: string
  colour: string
  image?: string
}

export interface Payload {
  title: string
  notes: string
  fields: Field[]
  icon?: RecordIcon | null
  alerts?: Alert[]
}

/** A row in the list: named, and carrying nothing secret. */
export type RecordPreview = RecordHeader & {
  title: string
  subtitle: string
  url: string
  icon: RecordIcon | null
  /** AWS only: a String/StringList parameter, which Keyhold shows but never overwrites. */
  plaintext?: boolean
}

export interface AttachmentInfo {
  id: string
  record_id: string
  name: string
  mime: string
  size: number
  created_at: string
}

export type OpenRecord = RecordHeader & { payload: Payload; plaintext?: boolean; /** An AWS record's parameter name. */ parameter?: string }

export interface VersionEntry {
  version: number
  created_at: string
}

export const api = {
  status: () => call<Status>('GET', '/api/status'),

  /** `password: null` means the vault is protected by the device key instead. */
  setup: (password: string | null) =>
    call<{ recoveryKey: string; lock: LockState }>('POST', '/api/setup', { password }),

  confirmRecovery: (recoveryKey: string) =>
    call<{ confirmed: boolean }>('POST', '/api/setup/recovery-confirmation', {
      recovery_key: recoveryKey,
    }),

  unlock: (attempt: { password?: string; code?: string; recovery_key?: string }) =>
    call<{ lock: LockState }>('POST', '/api/unlock', attempt),

  lock: () => call<{ lock: LockState }>('POST', '/api/lock'),

  heartbeat: () => call<{ lock: LockState }>('POST', '/api/heartbeat'),

  visibility: (visible: boolean) =>
    call<{ lock: LockState }>('POST', '/api/visibility', { visible }),

  setAppearance: (appearance: Appearance) =>
    call<{ appearance: Appearance }>('PUT', '/api/settings/appearance', appearance),

  setLockPolicy: (policy: LockPolicy) =>
    call<{ lock: LockState }>('PUT', '/api/settings/lock', policy),

  changeProtection: (password: string | null) =>
    call<{ changed: boolean }>('PUT', '/api/protection', { password }),

  beginSecondFactor: () =>
    call<{ secret: string; uri: string; qr: string }>('POST', '/api/second-factor/begin'),

  enableSecondFactor: (secret: string, code: string) =>
    call<{ enabled: boolean }>('POST', '/api/second-factor', { secret, code }),

  disableSecondFactor: (code: string) =>
    call<{ enabled: boolean }>('DELETE', '/api/second-factor', { code }),

  vaults: () => call<{ vaults: VaultSummary[] }>('GET', '/api/vaults'),

  createVault: (name: string, colour = 'brand', aws?: AwsVault, icon?: RecordIcon | null) =>
    call<{ vault: VaultSummary }>('POST', '/api/vaults', {
      name,
      description: '',
      colour,
      ...(icon ? { icon } : {}),
      ...(aws ? { kind: 'aws', aws } : {}),
    }),

  aws: () => call<{ logins: AwsLogin[]; connections: AwsConnection[] }>('GET', '/api/aws'),

  startAwsLogin: (startUrl: string, region: string, label: string) =>
    call<{ login: AwsLogin; device: DeviceLogin | null }>('POST', '/api/aws/logins', {
      start_url: startUrl,
      region,
      label,
    }),

  pollAwsLogin: (id: string) =>
    call<{ state: 'pending' | 'slow_down' | 'done' | 'denied' | 'expired' }>('POST', `/api/aws/logins/${id}/poll`),

  renewAwsLogin: (id: string) =>
    call<{ login: AwsLogin; device: DeviceLogin | null }>('POST', `/api/aws/logins/${id}/renew`),

  revokeAwsLogin: (id: string) => call<{ revoked: boolean }>('POST', `/api/aws/logins/${id}/revoke`),

  removeAwsLogin: (id: string) => call<{ removed: boolean }>('DELETE', `/api/aws/logins/${id}`),

  awsAccounts: (loginId: string) =>
    call<{ accounts: Array<{ account_id: string; account_name: string; roles: string[] }> }>(
      'GET',
      `/api/aws/logins/${loginId}/accounts`,
    ),

  addAwsConnection: (connection: Omit<AwsConnection, 'id' | 'used_by'>) =>
    call<{ connection: AwsConnection }>('POST', '/api/aws/connections', connection),

  updateAwsConnection: (id: string, change: { role: string; title: string; note: string }) =>
    call<{ connection: AwsConnection }>('PUT', `/api/aws/connections/${id}`, change),

  removeAwsConnection: (id: string) => call<{ removed: boolean }>('DELETE', `/api/aws/connections/${id}`),

  checkAwsVault: (vaultId: string) =>
    call<{ parameters: boolean; bucket: 'ok' | 'missing' | 'denied' | null; message: string }>(
      'POST',
      `/api/aws/vaults/${vaultId}/check`,
    ),

  updateVault: (
    id: string,
    meta: { name: string; description: string; colour: string; icon?: RecordIcon | null; aws?: AwsVault | null },
  ) =>
    call<{ vault: VaultSummary }>('PUT', `/api/vaults/${id}`, meta),

  archiveVault: (id: string, on: boolean) =>
    call<{ archived: boolean }>('POST', `/api/vaults/${id}/archive`, { on }),

  setAgentAccess: (id: string, on: boolean) =>
    call<{ agent_access: boolean }>('POST', `/api/vaults/${id}/agent-access`, { on }),

  folders: (vaultId: string) =>
    call<{ folders: FolderNode[] }>('GET', `/api/vaults/${vaultId}/folders`),

  /** `segment`: an AWS folder's path segment, as shown in "Na AWS". */
  createFolder: (vaultId: string, name: string, parentId: string | null, colour = 'default', segment?: string) =>
    call<{ folder: FolderNode }>('POST', `/api/vaults/${vaultId}/folders`, {
      name,
      colour,
      parent_id: parentId,
      ...(segment ? { segment } : {}),
    }),

  /** `segment`: a new path for an AWS folder, which moves everything in it. */
  renameFolder: (id: string, name: string, colour: string | null, segment?: string) =>
    call<{ folder: FolderNode; warning?: string }>('PUT', `/api/folders/${id}`, {
      name,
      colour,
      ...(segment ? { segment } : {}),
    }),

  moveFolder: (id: string, parentId: string | null) =>
    call<{ moved: boolean }>('POST', `/api/folders/${id}/move`, { parent_id: parentId }),

  deleteFolder: (id: string) => call<{ deleted: boolean }>('DELETE', `/api/folders/${id}`),

  records: (vaultId: string, archived = false) =>
    call<{ records: RecordPreview[] }>(
      'GET',
      `/api/vaults/${vaultId}/records?archived=${archived}`,
    ),

  /** `name`: an AWS record's parameter name (its last segment), as shown in "Na AWS". */
  createRecord: (vaultId: string, folderId: string | null, payload: Payload, name?: string) =>
    call<{ record: RecordHeader }>('POST', `/api/vaults/${vaultId}/records`, {
      folder_id: folderId,
      payload,
      ...(name ? { name } : {}),
    }),

  /** The code a 2FA field gives right now; the secret itself never needs to be shown. */
  totpCode: (recordId: string, index: number) =>
    call<{ code: string; remaining: number }>('GET', `/api/records/${recordId}/totp/${index}`),

  openRecord: (id: string) => call<{ record: OpenRecord }>('GET', `/api/records/${id}`),

  /**
   * `expectedVersion` makes the save refuse (409) when somebody else changed the record in the
   * meantime, which only happens in an AWS vault shared with other people.
   */
  saveRecord: (id: string, payload: Payload, expectedVersion?: number) =>
    call<{ record: RecordHeader; warning?: string }>('PUT', `/api/records/${id}`, {
      ...payload,
      ...(expectedVersion !== undefined ? { expected_version: expectedVersion } : {}),
    }),

  /**
   * Move a record. Passing `vaultId` moves it to another vault, which re-encrypts it — the folder
   * then has to be one of that vault's, or nothing.
   */
  /** `name`: a new last segment for an AWS record's parameter (a rename). */
  moveRecord: (id: string, folderId: string | null, vaultId?: string, name?: string) =>
    call<{ moved: boolean; id?: string; warning?: string; history_lost?: boolean }>('POST', `/api/records/${id}/move`, {
      folder_id: folderId,
      ...(vaultId ? { vault_id: vaultId } : {}),
      ...(name ? { name } : {}),
    }),

  favouriteRecord: (id: string, on: boolean) =>
    call<{ favourite: boolean }>('POST', `/api/records/${id}/favourite`, { on }),

  archiveRecord: (id: string, on: boolean) =>
    call<{ archived: boolean }>('POST', `/api/records/${id}/archive`, { on }),

  setCompromised: (id: string, on: boolean) =>
    call<{ compromised: boolean }>('POST', `/api/records/${id}/compromised`, { on }),

  versions: (id: string) =>
    call<{ versions: VersionEntry[] }>('GET', `/api/records/${id}/versions`),

  readVersion: (id: string, version: number) =>
    call<{ payload: Payload }>('GET', `/api/records/${id}/versions/${version}`),

  restoreVersion: (id: string, version: number, expectedVersion?: number) =>
    call<{ record: RecordHeader; warning?: string }>(
      'POST',
      `/api/records/${id}/versions/${version}/restore`,
      expectedVersion !== undefined ? { expected_version: expectedVersion } : undefined,
    ),

  attachments: (recordId: string) =>
    call<{ attachments: AttachmentInfo[]; max_bytes: number }>(
      'GET',
      `/api/records/${recordId}/attachments`,
    ),

  addAttachment: (recordId: string, name: string, mime: string, data: string) =>
    call<{ attachment: AttachmentInfo }>('POST', `/api/records/${recordId}/attachments`, {
      name,
      mime,
      data,
    }),

  openAttachment: (id: string) =>
    call<{ name: string; mime: string; data: string }>('GET', `/api/attachments/${id}`),

  deleteAttachment: (id: string) =>
    call<{ deleted: boolean }>('DELETE', `/api/attachments/${id}`),

  exportVault: (vaultId: string, passphrase: string) =>
    call<{ archive: unknown }>('POST', `/api/vaults/${vaultId}/export`, { passphrase }),

  importArchive: (vaultId: string, envelope: unknown, passphrase: string) =>
    call<{ imported: { records: number; folders: number } }>(
      'POST',
      `/api/vaults/${vaultId}/import`,
      { from: 'archive', envelope, passphrase },
    ),

  importCsv: (vaultId: string, text: string) =>
    call<{ imported: { records: number; folders: number } }>(
      'POST',
      `/api/vaults/${vaultId}/import`,
      { from: 'csv', text },
    ),

  agentStatus: () =>
    call<{
      listening: boolean
      socket: string
      exposed_vaults: number
      may_reveal: boolean
      can_copy: boolean
      active_grants: number
      command: string
    }>('GET', '/api/agent'),

  setAgentMayReveal: (on: boolean) =>
    call<{ may_reveal: boolean }>('PUT', '/api/agent/reveal', { on }),

  setAgentSurface: (on: boolean) =>
    call<{ listening: boolean }>('POST', '/api/agent', { on }),

  /** `looking` tells the server somebody has the window in front of them, so it need not also ask
   *  on the desktop. */
  agentRequests: (looking = false) =>
    call<{ requests: AgentRequest[]; active_grants: number }>(
      'GET',
      looking ? '/api/agent/requests?looking=true' : '/api/agent/requests',
    ),

  answerAgentRequest: (id: string, allowed: boolean, window: GrantWindow = { kind: 'once' }) =>
    call<{ answered: boolean }>('POST', `/api/agent/requests/${id}`, { allowed, window }),

  agentGrants: () => call<{ grants: AgentGrant[] }>('GET', '/api/agent/grants'),

  revokeAgentGrant: (id: string) =>
    call<{ revoked: boolean }>('DELETE', `/api/agent/grants/${id}`),

  revokeAgentGrants: () => call<{ active_grants: number }>('DELETE', '/api/agent/grants'),

  instances: () =>
    call<{ processes: InstanceProcess[]; version: string }>('GET', '/api/instances'),

  closeInstance: (pid: number) =>
    call<{ processes: InstanceProcess[] }>('POST', `/api/instances/${pid}/close`),

  audit: (limit = 200) =>
    call<{ events: AuditEvent[]; log_reads: boolean }>('GET', `/api/audit?limit=${limit}`),

  auditIntegrity: () =>
    call<{ integrity: AuditIntegrity }>('GET', '/api/audit/integrity'),

  setLogReads: (on: boolean) =>
    call<{ log_reads: boolean }>('PUT', '/api/settings/log-reads', { on }),

  /**
   * Tell the binary that a secret was looked at.
   *
   * Only the field's label travels — never its value. The binary cannot infer this on its own: it
   * hands over a whole record when a tab opens and has no idea which field was uncovered.
   */
  reportAccess: (recordId: string, kind: 'reveal' | 'copy', field: string) =>
    call<{ noted: boolean }>('POST', `/api/records/${recordId}/accesses`, { kind, field }),

  vaultContents: (vaultId: string) =>
    call<{ contents: { records: number; archived_records: number; folders: number } }>(
      'GET',
      `/api/vaults/${vaultId}/contents`,
    ),

  deleteVault: (vaultId: string, confirmName: string) =>
    call<{ deleted: { records: number; archived_records: number; folders: number } }>(
      'DELETE',
      `/api/vaults/${vaultId}`,
      { confirm_name: confirmName },
    ),

  health: (vaultId: string) =>
    call<{ health: VaultHealth }>('GET', `/api/vaults/${vaultId}/health`),

  rotateVault: (vaultId: string) =>
    call<{ vault: VaultSummary }>('POST', `/api/vaults/${vaultId}/rotate`),

  trash: (vaultId: string) =>
    call<{ records: RecordPreview[]; retention_days: number }>(
      'GET',
      `/api/vaults/${vaultId}/trash`,
    ),

  destroyRecord: (id: string) =>
    call<{ destroyed: boolean }>('POST', `/api/records/${id}/destroy`),

  emptyTrash: (vaultId: string) =>
    call<{ destroyed: number }>('DELETE', `/api/vaults/${vaultId}/trash`),

  setRetention: (days: number) =>
    call<{ retention_days: number }>('PUT', '/api/settings/retention', { days }),

  setRotationDue: (id: string, dueAt: string | null) =>
    call<{ rotation_due_at: string | null }>('POST', `/api/records/${id}/rotation`, {
      due_at: dueAt,
    }),

  generatePassword: (length = 20) =>
    call<{ password: string; entropyBits: number }>('POST', '/api/password', { ...DEFAULT_RECIPE, length }),

  generate: (recipe: Recipe) => call<{ password: string; entropyBits: number }>('POST', '/api/password', recipe),

  saveGenerator: (recipe: Recipe) => call<{ generator: Recipe }>('PUT', '/api/settings/generator', recipe),

  saveLayout: (layout: Layout) => call<{ layout: Layout }>('PUT', '/api/settings/layout', layout),

  alertPresets: () => call<{ presets: AlertPreset[] }>('GET', '/api/alerts'),

  saveAlertPresets: (presets: AlertPreset[]) => call<{ presets: AlertPreset[] }>('PUT', '/api/alerts', { presets }),

  copyPlan: (scope: CopyScope) => call<CopyPlan>('POST', '/api/copy/plan', scope),

  copyRun: (scope: CopyScope, item: CopyRunItem) =>
    call<CopyRunAnswer>('POST', '/api/copy/run', {
      source_vault: scope.source_vault,
      dest_vault: scope.dest_vault,
      dest_folder: scope.dest_folder ?? null,
      item,
    }),

  copyOriginals: (sourceVault: string, recordIds: string[], confirmName?: string) =>
    call<{ removed: number; skipped: number; failed: Array<{ id: string; error: string }>; trash: boolean }>(
      'POST',
      '/api/copy/originals',
      { source_vault: sourceVault, record_ids: recordIds, confirm_name: confirmName ?? null },
    ),
}
