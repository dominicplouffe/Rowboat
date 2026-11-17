import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';
import { applyCachedTheme } from './lib/theme';

applyCachedTheme();

export default mount(App, { target: document.getElementById('app')! });
