import { authHeaders } from './session'

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
  attemptsLeft: number
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

  moveRecord: (id: string, folderId: string | null) =>
    call<{ moved: boolean }>('POST', `/api/records/${id}/move`, { folder_id: folderId }),

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
