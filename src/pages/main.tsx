import React from 'react';
import ReactDOM from 'react-dom/client';
import '../lib/tauriApi';
import { VoiceEngine } from './VoiceEngine';
import '../styles.css';

ReactDOM.createRoot(document.getElementById('root')!).render(<VoiceEngine />);
