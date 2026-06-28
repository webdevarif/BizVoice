import React from 'react';
import ReactDOM from 'react-dom/client';
import '../lib/tauriApi'; // installs window.api under Tauri; no-op under Electron
import { Settings } from './Settings';
import '../styles.css';

ReactDOM.createRoot(document.getElementById('root')!).render(<Settings />);
