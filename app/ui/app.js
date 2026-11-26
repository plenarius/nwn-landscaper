// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

// The app's own page: asks the app where it is (`status`) until the engine answers, then goes to the editor.

const { invoke } = window.__TAURI__.core;
const $ = id => document.getElementById(id);

function show(section) {
    for (const id of ['starting', 'setup', 'failed']) $(id).hidden = id !== section;
}

let filled = false;

async function poll() {
    const s = await invoke('status');
    if (s.phase === 'ready') {
        location.replace(s.url);
        return;
    }
    if (s.phase === 'setup') {
        show('setup');
        if (!filled) {
            filled = true;
            $('root').value = s.root ?? '';
            $('user').value = s.user ?? '';
            $('mac').hidden = !navigator.userAgent.includes('Mac');
            if (s.problem) {
                $('problem').textContent = s.problem;
                $('problem').hidden = false;
            }
        }
    } else if (s.phase === 'failed') {
        show('failed');
        $('why').textContent = s.message;
        if (s.log) {
            $('logAt').textContent = `The log: ${s.log}`;
            $('logAt').hidden = false;
        }
        return;
    } else {
        show('starting');
    }
    setTimeout(poll, 250);
}

async function pick(input, title) {
    const folder = await invoke('pick_folder', { title });
    if (folder) $(input).value = folder;
}

$('pickRoot').addEventListener('click', () => pick('root', "The game's folder"));
$('pickUser').addEventListener('click', () => pick('user', 'Your user folder'));
$('go').addEventListener('click', async () => {
    $('problem').hidden = true;
    try {
        await invoke('use_folders', { root: $('root').value, user: $('user').value || null });
        show('starting');
    } catch (e) {
        $('problem').textContent = String(e);
        $('problem').hidden = false;
    }
});

poll();
