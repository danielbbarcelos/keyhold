/**
 * The English catalog, one file per area so each part of the interface keeps its own list.
 * Keys are the Portuguese text exactly as written in `t()`.
 */
import { APP } from './app'
import { CORE } from './core'
import { SETTINGS } from './settings'
import { VAULT } from './vault'

export const EN: Record<string, string> = { ...CORE, ...APP, ...SETTINGS, ...VAULT }
