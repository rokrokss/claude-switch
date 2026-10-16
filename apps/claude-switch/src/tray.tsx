// SPDX-License-Identifier: MIT

import React from 'react'

import ReactDOM from 'react-dom/client'

import { TrayPanel } from '@/features/tray/components/tray-panel'

import './index.css'

// The menu bar panel's own page (tray.html), loaded into the hidden `tray`
// window at launch: the main app's page would bring its dialogs and polling.
ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <TrayPanel />
  </React.StrictMode>,
)
