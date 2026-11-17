import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';
import { applyCachedTheme } from './lib/theme';

applyCachedTheme();

// Start fetching the editor now, in parallel with the first requests to the
// core, instead of after them. It stays out of the initial bundle and never
// blocks the first paint; the Editor component awaits the same module.
void import('./editor/sqlEditor');

export default mount(App, { target: document.getElementById('app')! });
