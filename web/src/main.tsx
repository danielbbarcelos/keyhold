import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'

import { App } from './App'
import { adoptTokenFromLocation } from './session'
import './index.css'

// Before anything renders: take the token out of the fragment and into memory. Every call the
// interface makes depends on it, and there is no second place to look for it.
adoptTokenFromLocation()

// Grafite until the binary says otherwise: the default theme, and the one that avoids a white flash
// before the first status arrives. `applyAppearance` takes over from there.
document.documentElement.classList.add('dark')

const root = document.getElementById('root')

if (!root) {
  throw new Error('the document is missing its root element')
}

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
