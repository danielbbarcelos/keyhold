import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'

import { App } from './App'
import { adoptSession } from './session'
import './index.css'


// Grafite until the binary says otherwise: the default theme, and the one that avoids a white flash
// before the first status arrives. `applyAppearance` takes over from there.
document.documentElement.classList.add('dark')

const root = document.getElementById('root')

if (!root) {
  throw new Error('the document is missing its root element')
}

// Before anything renders: trade the code in the address for the session cookie, and learn whether
// this tab has a session at all. Every screen depends on the answer.
void adoptSession().then(() => {
  createRoot(root).render(
    <StrictMode>
      <App />
    </StrictMode>,
  )
})
