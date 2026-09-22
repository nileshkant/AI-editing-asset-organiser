import React from 'react';
import ReactDOM from 'react-dom/client';
import { App } from './App';
import './styles/base.css';
import './styles/layout.css';
import './styles/components.css';
import './styles/sound-list.css';
import './styles/transport.css';
import './styles/waveform.css';
import './styles/responsive.css';

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
