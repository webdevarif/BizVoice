import React from 'react';
import ReactDOM from 'react-dom/client';
import '../lib/tauriApi'; // installs window.api under Tauri; no-op under Electron
import { Login } from './Login';
import '../styles.css';

ReactDOM.createRoot(document.getElementById('root')!).render(<Login />);
