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
  attemptsLeft: number
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

    throw new ApiError(response.status, payload.error ?? `${path} answered ${response.status}`)
  }

  return payload as T
}

export interface VaultSummary {
  id: string
  name: string
  description: string
  colour: string
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
  created_at: string
  updated_at: string
}

export interface Field {
  label: string
  kind: 'password' | 'text' | 'hidden' | 'url' | 'date'
  value: string
}

export interface Payload {
  title: string
  username: string
  url: string
  notes: string
  fields: Field[]
}

/** A row in the list: named, and carrying nothing secret. */
export type RecordPreview = RecordHeader & { title: string; username: string; url: string }

export interface AttachmentInfo {
  id: string
  record_id: string
  name: string
  mime: string
  size: number
  created_at: string
}

export type OpenRecord = RecordHeader & { payload: Payload }

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

  createVault: (name: string, colour = 'brand') =>
    call<{ vault: VaultSummary }>('POST', '/api/vaults', { name, description: '', colour }),

  updateVault: (id: string, meta: { name: string; description: string; colour: string }) =>
    call<{ vault: VaultSummary }>('PUT', `/api/vaults/${id}`, meta),

  archiveVault: (id: string, on: boolean) =>
    call<{ archived: boolean }>('POST', `/api/vaults/${id}/archive`, { on }),

  setAgentAccess: (id: string, on: boolean) =>
    call<{ agent_access: boolean }>('POST', `/api/vaults/${id}/agent-access`, { on }),

  folders: (vaultId: string) =>
    call<{ folders: FolderNode[] }>('GET', `/api/vaults/${vaultId}/folders`),

  createFolder: (vaultId: string, name: string, parentId: string | null, colour = 'default') =>
    call<{ folder: FolderNode }>('POST', `/api/vaults/${vaultId}/folders`, {
      name,
      colour,
      parent_id: parentId,
    }),

  renameFolder: (id: string, name: string, colour: string | null) =>
    call<{ folder: FolderNode }>('PUT', `/api/folders/${id}`, { name, colour }),

  moveFolder: (id: string, parentId: string | null) =>
    call<{ moved: boolean }>('POST', `/api/folders/${id}/move`, { parent_id: parentId }),

  deleteFolder: (id: string) => call<{ deleted: boolean }>('DELETE', `/api/folders/${id}`),

  records: (vaultId: string, archived = false) =>
    call<{ records: RecordPreview[] }>(
      'GET',
      `/api/vaults/${vaultId}/records?archived=${archived}`,
    ),

  createRecord: (vaultId: string, kind: string, folderId: string | null, payload: Payload) =>
    call<{ record: RecordHeader }>('POST', `/api/vaults/${vaultId}/records`, {
      kind,
      folder_id: folderId,
      payload,
    }),

  openRecord: (id: string) => call<{ record: OpenRecord }>('GET', `/api/records/${id}`),

  saveRecord: (id: string, payload: Payload) =>
    call<{ record: RecordHeader }>('PUT', `/api/records/${id}`, payload),

  /**
   * Move a record. Passing `vaultId` moves it to another vault, which re-encrypts it — the folder
   * then has to be one of that vault's, or nothing.
   */
  moveRecord: (id: string, folderId: string | null, vaultId?: string) =>
    call<{ moved: boolean }>('POST', `/api/records/${id}/move`, {
      folder_id: folderId,
      ...(vaultId ? { vault_id: vaultId } : {}),
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

  restoreVersion: (id: string, version: number) =>
    call<{ record: RecordHeader }>('POST', `/api/records/${id}/versions/${version}/restore`),

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

  agentRequests: () =>
    call<{ requests: AgentRequest[]; active_grants: number }>('GET', '/api/agent/requests'),

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
    call<{ password: string; entropyBits: number }>('POST', '/api/password', {
      length,
      lowercase: true,
      uppercase: true,
      digits: true,
      symbols: true,
      avoid_ambiguous: false,
    }),
}
