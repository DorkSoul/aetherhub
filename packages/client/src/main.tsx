import '@fontsource/fraunces/500.css';
import '@fontsource/fraunces/600.css';
import '@fontsource/fraunces/700.css';
import '@fontsource/ibm-plex-sans/400.css';
import '@fontsource/ibm-plex-sans/500.css';
import '@fontsource/ibm-plex-sans/600.css';
import '@fontsource/ibm-plex-mono/400.css';
import '@fontsource/ibm-plex-mono/600.css';
import './styles/base.css';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import { applyCachedAccent } from './lib/store';

applyCachedAccent();
createRoot(document.getElementById('root')!).render(<App />);

// Installable as an app (Add to Home screen → opens fullscreen). Relative to the page's <base>, so it works under BASE_PATH.
if (import.meta.env.PROD && 'serviceWorker' in navigator) void navigator.serviceWorker.register('sw.js').catch(() => {});
