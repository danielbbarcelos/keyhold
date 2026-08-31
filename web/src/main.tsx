import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'

import { App } from './App'
import { adoptTokenFromLocation } from './session'
import './index.css'

// Before anything renders: take the token out of the fragment and into memory. Every call the
// interface makes depends on it, and there is no second place to look for it.
adoptTokenFromLocation()

// keyhold is dark, and only dark. A vault is a tool you keep open beside other work rather than
// something you read, and a theme switch is one more decision to hand somebody who came here to
// look up a password.
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
