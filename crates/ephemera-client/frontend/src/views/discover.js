/**
 * Ephemera -- Discover View
 *
 * Combined search/people discovery with:
 * - Search bar for finding people
 * - QR code connection modal
 * - Connect by pseudonym ID
 * - Pending/active connections management (prominent accept/reject)
 * - Browse Groups shortcut section
 * - Mentions indicator
 * - Rich empty states
 *
 * RPC calls:
 *   - social.list_connections { status }
 *   - social.accept { from }
 *   - social.reject { from }
 *   - social.connect { target, message }
 *   - social.cancel_request { target }
 *   - social.resend_request { target, message }
 *   - social.disconnect { target }
 *   - network.connect { node_id }
 *   - identity.invite_qr {}
 *   - identity.lookup_handle { name }
 *   - groups.search { query }
 *   - mentions.list { limit }
 */
(function () {
    'use strict';

    function renderConnectionCard(conn, parentContainer) {
        var card = Ephemera.el('div', 'connection-card');
        card.setAttribute('role', 'listitem');

        var displayName = conn.display_name || 'Anonymous';
        card.appendChild(Ephemera.avatar(displayName, 'avatar-md', conn.avatar_url || null));

        var info = Ephemera.el('div', 'connection-info');
        info.appendChild(Ephemera.el('div', 'connection-name', displayName));

        if (conn.handle) {
            info.appendChild(Ephemera.el('div', 'connection-handle', '@' + conn.handle));
        } else if (conn.pseudonym_id) {
            var handleText = conn.pseudonym_id.length > 16
                ? '@' + conn.pseudonym_id.slice(0, 8) + '...' + conn.pseudonym_id.slice(-4)
                : '@' + conn.pseudonym_id;
            info.appendChild(Ephemera.el('div', 'connection-handle', handleText));
        }

        if (conn.message && conn.status === 'pending_incoming') {
            var msgEl = Ephemera.el('div', 'connection-mutual');
            msgEl.style.fontStyle = 'italic';
            msgEl.textContent = '"' + conn.message + '"';
            info.appendChild(msgEl);
        }

        card.appendChild(info);

        if (conn.status === 'pending_incoming') {
            card.classList.add('pending-incoming');
            var actions = Ephemera.el('div', 'connection-actions');

            var acceptBtn = Ephemera.el('button', 'btn btn-primary btn-sm', 'Accept');
            acceptBtn.setAttribute('aria-label', 'Accept ' + displayName);
            acceptBtn.addEventListener('click', async function () {
                acceptBtn.disabled = true;
                try {
                    await Ephemera.rpc('social.accept', { from: conn.pseudonym_id });
                    Ephemera.showToast('Connected!', 'success');
                    renderDiscover(parentContainer);
                } catch (err) {
                    Ephemera.showToast('Failed: ' + err.message, 'error');
                    acceptBtn.disabled = false;
                }
            });
            actions.appendChild(acceptBtn);

            var rejectBtn = Ephemera.el('button', 'btn btn-ghost btn-sm', 'Reject');
            rejectBtn.setAttribute('aria-label', 'Reject ' + displayName);
            rejectBtn.addEventListener('click', async function () {
                rejectBtn.disabled = true;
                try {
                    await Ephemera.rpc('social.reject', { from: conn.pseudonym_id });
                    card.style.opacity = '0';
                    card.style.transition = 'opacity 300ms ease-out';
                    setTimeout(function () { card.remove(); }, 300);
                    Ephemera.showToast('Request rejected', 'info');
                } catch (err) {
                    Ephemera.showToast('Failed: ' + err.message, 'error');
                    rejectBtn.disabled = false;
                }
            });
            actions.appendChild(rejectBtn);
            card.appendChild(actions);

        } else if (conn.status === 'pending_outgoing') {
            var outActions = Ephemera.el('div', 'connection-actions');

            var resendBtn = Ephemera.el('button', 'btn btn-primary btn-sm', 'Resend');
            resendBtn.setAttribute('aria-label', 'Resend request to ' + displayName);
            resendBtn.addEventListener('click', async function () {
                resendBtn.disabled = true;
                resendBtn.textContent = 'Sending...';
                try {
                    await Ephemera.rpc('social.resend_request', {
                        target: conn.pseudonym_id,
                        message: conn.message || 'Hi! I\'d like to connect.',
                    });
                    Ephemera.showToast('Request resent', 'success');
                    renderDiscover(parentContainer);
                } catch (err) {
                    Ephemera.showToast('Failed: ' + err.message, 'error');
                    resendBtn.disabled = false;
                    resendBtn.textContent = 'Resend';
                }
            });
            outActions.appendChild(resendBtn);

            var cancelBtn = Ephemera.el('button', 'btn btn-ghost btn-sm', 'Cancel');
            cancelBtn.setAttribute('aria-label', 'Cancel request to ' + displayName);
            cancelBtn.addEventListener('click', async function () {
                cancelBtn.disabled = true;
                try {
                    await Ephemera.rpc('social.cancel_request', { target: conn.pseudonym_id });
                    card.style.opacity = '0';
                    card.style.transition = 'opacity 300ms ease-out';
                    setTimeout(function () { card.remove(); }, 300);
                    Ephemera.showToast('Request cancelled', 'info');
                } catch (err) {
                    Ephemera.showToast('Failed: ' + err.message, 'error');
                    cancelBtn.disabled = false;
                }
            });
            outActions.appendChild(cancelBtn);

            var pendingLabel = Ephemera.el('span', '');
            pendingLabel.style.cssText = 'font-size:var(--fs-sm);color:var(--text-tertiary);flex-shrink:0;margin-right:8px;';
            pendingLabel.textContent = 'Pending';
            card.appendChild(pendingLabel);
            card.appendChild(outActions);

        } else if (conn.status === 'connected') {
            var removeBtn = Ephemera.el('button', 'btn btn-ghost btn-sm', 'Remove');
            removeBtn.setAttribute('aria-label', 'Remove ' + displayName);
            removeBtn.addEventListener('click', async function () {
                if (!confirm('Remove connection with ' + displayName + '?')) return;
                removeBtn.disabled = true;
                try {
                    await Ephemera.rpc('social.disconnect', { target: conn.pseudonym_id });
                    card.style.opacity = '0';
                    card.style.transition = 'opacity 300ms ease-out';
                    setTimeout(function () { card.remove(); }, 300);
                    Ephemera.showToast('Connection removed', 'info');
                } catch (err) {
                    Ephemera.showToast('Failed: ' + err.message, 'error');
                    removeBtn.disabled = false;
                }
            });
            card.appendChild(removeBtn);
        }

        return card;
    }

    function showQrModal() {
        var overlay = Ephemera.el('div', 'modal-overlay');
        overlay.setAttribute('role', 'dialog');
        overlay.setAttribute('aria-modal', 'true');
        overlay.setAttribute('aria-label', 'Share your invite code');

        var modal = Ephemera.el('div', 'modal-content');
        modal.style.textAlign = 'center';

        modal.appendChild(Ephemera.el('h2', '', 'Share Your Invite'));
        modal.appendChild(Ephemera.el('p', '', 'Show the QR code, or share the link below.'));

        // -- QR code image --
        var qrEl = Ephemera.el('div', 'qr-placeholder', 'Loading QR...');
        qrEl.setAttribute('role', 'img');
        qrEl.setAttribute('aria-label', 'Your connection QR code');
        modal.appendChild(qrEl);

        // We store the invite link text once loaded so the copy button can use it.
        var inviteLink = '';

        loadQrCode(qrEl).then(function (link) {
            if (link) inviteLink = link;
        });

        // -- Visible invite link text (selectable) --
        var linkDisplay = Ephemera.el('div', '');
        linkDisplay.style.cssText = 'margin:12px 0 4px;padding:8px 12px;background:var(--bg-tertiary,#222);' +
            'border-radius:var(--radius-sm,4px);font-family:var(--font-mono,monospace);font-size:0.78rem;' +
            'word-break:break-all;color:var(--text-secondary);user-select:all;max-height:56px;overflow-y:auto;';
        linkDisplay.textContent = 'Loading...';
        modal.appendChild(linkDisplay);

        // Update link display once identity is available.
        var state = Ephemera.store.get();
        var identity = state.identity || {};
        var pubkey = identity.pubkey || identity.pseudonym_id || identity.public_key || '';
        if (pubkey) {
            inviteLink = 'ephemera://connect/' + pubkey;
            linkDisplay.textContent = inviteLink;
        }

        // -- Copy Link button --
        var copyBtn = Ephemera.el('button', 'btn btn-secondary btn-full btn-sm', 'Copy Invite Link');
        copyBtn.style.marginTop = '8px';
        copyBtn.addEventListener('click', function () {
            var link = inviteLink || linkDisplay.textContent;
            if (navigator.clipboard && navigator.clipboard.writeText) {
                navigator.clipboard.writeText(link).then(function () {
                    copyBtn.textContent = 'Copied!';
                    Ephemera.showToast('Invite link copied!', 'success');
                    setTimeout(function () { copyBtn.textContent = 'Copy Invite Link'; }, 2000);
                }).catch(function () {
                    Ephemera.showToast('Copy failed -- select the text above manually', 'error');
                });
            } else {
                // Fallback: select the text for manual copy
                var range = document.createRange();
                range.selectNodeContents(linkDisplay);
                var sel = window.getSelection();
                sel.removeAllRanges();
                sel.addRange(range);
                Ephemera.showToast('Select the text and copy manually', 'info');
            }
        });
        modal.appendChild(copyBtn);

        // -- Divider --
        var divider = Ephemera.el('div', '');
        divider.style.cssText = 'margin:16px 0 12px;border-top:1px solid var(--border,#333);' +
            'text-align:center;position:relative;';
        var dividerText = Ephemera.el('span', '');
        dividerText.style.cssText = 'position:relative;top:-0.7em;background:var(--bg-secondary,#1a1a1a);' +
            'padding:0 12px;font-size:0.8rem;color:var(--text-tertiary);';
        dividerText.textContent = 'or connect with their link';
        divider.appendChild(dividerText);
        modal.appendChild(divider);

        // -- Paste / manual entry input --
        var pasteRow = Ephemera.el('div', '');
        pasteRow.style.cssText = 'display:flex;gap:8px;';

        var pasteInput = document.createElement('input');
        pasteInput.type = 'text';
        pasteInput.className = 'input-field';
        pasteInput.placeholder = 'Paste invite link, pubkey, or @handle...';
        pasteInput.setAttribute('aria-label', 'Paste invite link or ID');
        pasteInput.style.flex = '1';
        pasteRow.appendChild(pasteInput);

        var pasteBtn = Ephemera.el('button', 'btn btn-primary btn-sm', 'Connect');
        pasteBtn.addEventListener('click', async function () {
            var target = pasteInput.value.trim();
            if (!target) {
                Ephemera.showToast('Paste an invite link, pubkey, or @handle', 'error');
                return;
            }
            pasteBtn.disabled = true;
            pasteBtn.textContent = 'Connecting...';
            try {
                var resolved = await resolvePubkey(target);
                if (resolved) {
                    await connectViaPubkey(resolved, 'Connected via invite link!');
                    // connectViaPubkey already shows step-by-step toasts
                    overlay.remove();
                }
            } catch (err) {
                Ephemera.showToast('Failed: ' + err.message, 'error');
            }
            pasteBtn.disabled = false;
            pasteBtn.textContent = 'Connect';
        });
        pasteRow.appendChild(pasteBtn);
        modal.appendChild(pasteRow);

        // Handle Enter key in paste input
        pasteInput.addEventListener('keydown', function (e) {
            if (e.key === 'Enter') pasteBtn.click();
        });

        // -- Tip for mobile users --
        var tip = Ephemera.el('p', '');
        tip.style.cssText = 'font-size:0.75rem;color:var(--text-tertiary);margin-top:12px;';
        tip.textContent = 'Tip: On mobile, you can use your phone\'s camera app to scan the QR code -- it will open Ephemera automatically.';
        modal.appendChild(tip);

        // -- Close button --
        var closeBtn = Ephemera.el('button', 'btn btn-ghost btn-full', 'Close');
        closeBtn.style.marginTop = '8px';
        closeBtn.addEventListener('click', function () { overlay.remove(); });
        modal.appendChild(closeBtn);

        overlay.appendChild(modal);

        overlay.addEventListener('click', function (e) {
            if (e.target === overlay) overlay.remove();
        });

        function onEsc(e) {
            if (e.key === 'Escape') { overlay.remove(); document.removeEventListener('keydown', onEsc); }
        }
        document.addEventListener('keydown', onEsc);

        document.body.appendChild(overlay);
    }

    /**
     * Load the QR code SVG into the given element.
     * Returns the invite link string on success, or null on failure.
     */
    async function loadQrCode(qrEl) {
        try {
            var result = await Ephemera.rpc('identity.invite_qr', {});
            if (result && result.qr_svg) {
                qrEl.innerHTML = result.qr_svg;
                var svg = qrEl.querySelector('svg');
                if (svg) {
                    svg.style.cssText = 'width:160px;height:160px;border-radius:var(--radius-sm);';
                    svg.setAttribute('role', 'img');
                    svg.setAttribute('aria-label', 'QR code');
                }
                return result.invite_link || null;
            } else {
                qrEl.textContent = 'QR not available';
                return null;
            }
        } catch (_e) {
            qrEl.textContent = 'QR not available';
            return null;
        }
    }

    /**
     * Parse a connection target string into a pubkey hex.
     * Accepts: ephemera:// invite links, @handle, #handle, or raw hex pubkey.
     * Returns the resolved pubkey hex, or null if lookup failed.
     */
    async function resolvePubkey(target) {
        var pubkey = target.trim();

        // Handle ephemera:// invite links (strip query params too)
        if (pubkey.startsWith('ephemera://connect/')) {
            pubkey = pubkey.replace('ephemera://connect/', '').split('?')[0].trim();
        }

        // Handle @handle lookup -- resolve to pubkey first
        if (pubkey.startsWith('@') || pubkey.startsWith('#')) {
            var handleName = pubkey.replace(/^[@#]/, '').toLowerCase().trim();
            var lookup = await Ephemera.rpc('identity.lookup_handle', { name: handleName });
            if (lookup && lookup.owner) {
                return lookup.owner;
            } else if (lookup && lookup.owner_pubkey) {
                return lookup.owner_pubkey;
            } else if (lookup && lookup.pubkey) {
                return lookup.pubkey;
            } else {
                Ephemera.showToast('Handle @' + handleName + ' not found', 'error');
                return null;
            }
        }

        return pubkey;
    }

    /**
     * Connect to a peer via pubkey: first establish network transport (Iroh),
     * then send a social connection request.
     *
     * Shows step-by-step status in a progress panel so the user can see
     * exactly what is happening at each stage of the connection process.
     * Essential for debugging on both desktop and phone.
     */
    async function connectViaPubkey(pubkey, message) {
        // Create a visible step-by-step progress panel
        var stepsContainer = document.querySelector('.connection-steps');
        if (!stepsContainer) {
            stepsContainer = Ephemera.el('div', 'connection-steps');
            // Insert after the connect row in the discover view
            var mainContent = document.getElementById('main-content');
            if (mainContent) {
                var connectRow = mainContent.querySelector('.discover-header');
                if (connectRow && connectRow.nextSibling) {
                    connectRow.parentNode.insertBefore(stepsContainer, connectRow.nextSibling.nextSibling);
                } else if (mainContent.firstChild) {
                    mainContent.insertBefore(stepsContainer, mainContent.firstChild.nextSibling);
                } else {
                    mainContent.appendChild(stepsContainer);
                }
            }
        }
        stepsContainer.innerHTML = '';

        var shortId = pubkey.length > 12 ? pubkey.slice(0, 8) + '...' : pubkey;

        function addStep(label, status) {
            var step = Ephemera.el('div', 'connection-step ' + status);
            var statusEl = Ephemera.el('span', 'step-status');
            if (status === 'pending') statusEl.textContent = '-';
            else if (status === 'active') statusEl.textContent = '...';
            else if (status === 'success') statusEl.textContent = 'OK';
            else if (status === 'error') statusEl.textContent = 'X';
            step.appendChild(statusEl);
            step.appendChild(Ephemera.el('span', '', label));
            stepsContainer.appendChild(step);
            return { el: step, statusEl: statusEl };
        }

        function updateStep(stepObj, newLabel, newStatus) {
            stepObj.el.className = 'connection-step ' + newStatus;
            if (newStatus === 'active') stepObj.statusEl.textContent = '...';
            else if (newStatus === 'success') stepObj.statusEl.textContent = 'OK';
            else if (newStatus === 'error') stepObj.statusEl.textContent = 'X';
            if (newLabel) {
                var msgSpan = stepObj.el.querySelector('span:last-child');
                if (msgSpan) msgSpan.textContent = newLabel;
            }
        }

        // Step 1: Check network status
        var step1 = addStep('[1/3] Checking network status...', 'active');
        var step2 = addStep('[2/3] Connecting to peer...', 'pending');
        var step3 = addStep('[3/3] Sending connection request...', 'pending');

        var networkOk = false;
        try {
            var netStatus = await Ephemera.rpc('network.status', {});
            var transportLabel = netStatus.iroh_available ? 'Iroh active' : (netStatus.transport || 'unknown');
            var nodeLabel = netStatus.node_id ? ' (node: ' + netStatus.node_id.slice(0, 8) + '...)' : '';
            updateStep(step1, '[1/3] Checking network status... ' + transportLabel + nodeLabel, 'success');
            networkOk = true;
        } catch (e) {
            updateStep(step1, '[1/3] Checking network status... Network unavailable', 'error');
        }

        // Step 2: Connect to peer via Iroh
        updateStep(step2, '[2/3] Connecting to peer... Searching via relay...', 'active');
        try {
            await Ephemera.rpc('network.connect', { node_id: pubkey });
            updateStep(step2, '[2/3] Connecting to peer... Connected!', 'success');
        } catch (e) {
            console.warn('Network connect (may be offline):', e.message || e);
            updateStep(step2, '[2/3] Connecting to peer... Peer not found (they may be offline)', 'error');
        }

        // Step 3: Send social connection request
        updateStep(step3, '[3/3] Saving connection request...', 'active');
        try {
            await Ephemera.rpc('social.connect', {
                target: pubkey,
                message: message || 'Hi! I\'d like to connect.',
            });
            updateStep(step3, '[3/3] Connection request sent!', 'success');
            Ephemera.showToast('Connection request sent!', 'success');
        } catch (err) {
            updateStep(step3, '[3/3] Failed to send request: ' + (err.message || err), 'error');
            throw err;
        }

        // Auto-remove the progress panel after 5 seconds
        setTimeout(function () {
            if (stepsContainer.parentNode) {
                stepsContainer.style.opacity = '0';
                stepsContainer.style.transition = 'opacity 300ms ease-out';
                setTimeout(function () {
                    if (stepsContainer.parentNode) stepsContainer.remove();
                }, 300);
            }
        }, 5000);
    }

    async function sendConnectionRequest(target) {
        try {
            var pubkey = await resolvePubkey(target);
            if (!pubkey) return;

            await connectViaPubkey(pubkey);
            // connectViaPubkey already shows step-by-step toasts
        } catch (err) {
            Ephemera.showToast('Failed: ' + err.message, 'error');
        }
    }

    // Track the polling interval so we can clear it when navigating away.
    var _discoverPollTimer = null;

    function stopDiscoverPolling() {
        if (_discoverPollTimer) {
            clearInterval(_discoverPollTimer);
            _discoverPollTimer = null;
        }
    }

    async function renderDiscover(container) {
        // Stop any previous polling interval (e.g. when re-rendering).
        stopDiscoverPolling();
        container.innerHTML = '';

        // Header
        var header = Ephemera.el('div', 'discover-header');
        header.appendChild(Ephemera.el('h1', '', 'Discover'));

        var qrBtn = Ephemera.el('button', 'btn btn-ghost btn-sm');
        qrBtn.innerHTML = '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2"><rect x="3" y="3" width="7" height="7"/><rect x="14" y="3" width="7" height="7"/><rect x="3" y="14" width="7" height="7"/><rect x="14" y="14" width="3" height="3"/><rect x="18" y="14" width="3" height="3"/><rect x="14" y="18" width="3" height="3"/><rect x="18" y="18" width="3" height="3"/></svg>';
        qrBtn.setAttribute('aria-label', 'Show QR code');
        qrBtn.addEventListener('click', showQrModal);
        header.appendChild(qrBtn);
        container.appendChild(header);

        // Search bar
        var searchBar = Ephemera.el('div', 'search-bar');
        searchBar.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/></svg>';
        var searchInput = document.createElement('input');
        searchInput.type = 'text';
        searchInput.className = 'input-field';
        searchInput.placeholder = 'Search people...';
        searchInput.setAttribute('aria-label', 'Search connections');
        searchBar.appendChild(searchInput);
        container.appendChild(searchBar);

        // Connect by ID section
        var connectRow = Ephemera.el('div', '');
        connectRow.style.cssText = 'display:flex;gap:8px;margin-bottom:24px;';

        var connectInput = document.createElement('input');
        connectInput.type = 'text';
        connectInput.className = 'input-field';
        connectInput.placeholder = 'Paste invite link, pseudonym ID, or @handle...';
        connectInput.setAttribute('aria-label', 'Pseudonym ID to connect');
        connectInput.style.flex = '1';
        connectRow.appendChild(connectInput);

        var connectBtn = Ephemera.el('button', 'btn btn-primary btn-sm', 'Connect');
        connectBtn.addEventListener('click', function () {
            var target = connectInput.value.trim();
            if (!target) {
                Ephemera.showToast('Enter a pseudonym ID or handle', 'error');
                return;
            }
            sendConnectionRequest(target);
            connectInput.value = '';
        });
        connectRow.appendChild(connectBtn);
        container.appendChild(connectRow);

        // Loading state
        var loading = Ephemera.el('div', 'loading-state');
        loading.setAttribute('role', 'status');
        loading.appendChild(Ephemera.el('div', 'spinner'));
        loading.appendChild(Ephemera.el('p', '', 'Loading connections...'));
        container.appendChild(loading);

        try {
            var result = await Ephemera.rpc('social.list_connections', { status: 'all' });
            container.removeChild(loading);

            var connections = result.connections || result || [];

            var pending = [];
            var active = [];
            var outgoing = [];

            if (Array.isArray(connections)) {
                connections.forEach(function (c) {
                    if (c.status === 'pending_incoming') pending.push(c);
                    else if (c.status === 'pending_outgoing') outgoing.push(c);
                    else active.push(c);
                });
            }

            // Prominent pending requests banner
            if (pending.length > 0) {
                var pendingBanner = Ephemera.el('div', 'pending-banner');
                pendingBanner.innerHTML = '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2"><path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><line x1="19" y1="8" x2="19" y2="14"/><line x1="22" y1="11" x2="16" y2="11"/></svg>';
                pendingBanner.appendChild(document.createTextNode(
                    ' ' + pending.length + ' pending connection request' +
                    (pending.length > 1 ? 's' : '') + ' -- respond below'));
                container.appendChild(pendingBanner);

                container.appendChild(Ephemera.el('div', 'section-title',
                    'Pending Requests (' + pending.length + ')'));
                var pendingList = Ephemera.el('div', '');
                pendingList.setAttribute('role', 'list');
                pending.forEach(function (c) {
                    pendingList.appendChild(renderConnectionCard(c, container));
                });
                container.appendChild(pendingList);
            }

            if (outgoing.length > 0) {
                container.appendChild(Ephemera.el('div', 'section-title',
                    'Sent Requests (' + outgoing.length + ')'));
                var outList = Ephemera.el('div', '');
                outList.setAttribute('role', 'list');
                outgoing.forEach(function (c) {
                    outList.appendChild(renderConnectionCard(c, container));
                });
                container.appendChild(outList);
            }

            if (active.length > 0) {
                container.appendChild(Ephemera.el('div', 'section-title',
                    'Connected (' + active.length + ')'));
                var activeList = Ephemera.el('div', '');
                activeList.setAttribute('role', 'list');
                active.forEach(function (c) {
                    activeList.appendChild(renderConnectionCard(c, container));
                });
                container.appendChild(activeList);
            }

            if (pending.length === 0 && active.length === 0 && outgoing.length === 0) {
                var empty = Ephemera.el('div', 'empty-state');

                var icon = Ephemera.el('div', 'empty-state-icon');
                icon.innerHTML = '<svg viewBox="0 0 24 24" width="64" height="64" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M23 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/></svg>';
                empty.appendChild(icon);

                empty.appendChild(Ephemera.el('h3', '', 'Add your first connection'));
                empty.appendChild(Ephemera.el('p', '',
                    'Share your QR code with someone nearby, or paste their pseudonym ID above.'));

                var qrButton = Ephemera.el('button', 'btn btn-primary', 'Show My QR Code');
                qrButton.addEventListener('click', showQrModal);
                empty.appendChild(qrButton);

                container.appendChild(empty);
            }

        } catch (err) {
            container.removeChild(loading);
            console.error('Connections load error:', err);

            var empty = Ephemera.el('div', 'empty-state');

            var icon = Ephemera.el('div', 'empty-state-icon');
            icon.innerHTML = '<svg viewBox="0 0 24 24" width="64" height="64" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/></svg>';
            empty.appendChild(icon);

            empty.appendChild(Ephemera.el('h3', '', 'Add your first connection'));
            empty.appendChild(Ephemera.el('p', '', 'Share your QR code or paste a pseudonym ID to connect.'));

            var qrButton = Ephemera.el('button', 'btn btn-primary', 'Show My QR Code');
            qrButton.addEventListener('click', showQrModal);
            empty.appendChild(qrButton);

            container.appendChild(empty);
        }

        // Mentions section
        renderMentionsSection(container);

        // Browse Groups shortcut
        renderGroupsShortcut(container);

        // Local search filtering
        searchInput.addEventListener('input', function () {
            var query = searchInput.value.toLowerCase();
            container.querySelectorAll('.connection-card').forEach(function (card) {
                var text = card.textContent.toLowerCase();
                card.style.display = text.indexOf(query) !== -1 ? '' : 'none';
            });
        });

        // Poll connections every 10 seconds to detect status changes
        // (e.g. pending_outgoing -> connected when the other side accepts).
        // Only re-render if a status actually changed.
        var lastConnectionSnapshot = JSON.stringify(connections);
        _discoverPollTimer = setInterval(async function () {
            // Skip if we navigated away from the discover view.
            if (!container.parentNode) {
                stopDiscoverPolling();
                return;
            }
            try {
                var freshResult = await Ephemera.rpc('social.list_connections', { status: 'all' });
                var freshConns = freshResult.connections || freshResult || [];
                var freshSnapshot = JSON.stringify(freshConns);
                if (freshSnapshot !== lastConnectionSnapshot) {
                    lastConnectionSnapshot = freshSnapshot;
                    // Re-render the discover view to reflect updated statuses.
                    renderDiscover(container);
                }
            } catch (_e) {
                // Silently ignore poll failures (network offline, etc.)
            }
        }, 10000);
    }

    // ================================================================
    // Mentions section
    // ================================================================

    async function renderMentionsSection(container) {
        try {
            var result = await Ephemera.rpc('mentions.list', { limit: 5 });
            var mentions = result.mentions || result.items || [];
            if (!Array.isArray(mentions) || mentions.length === 0) return;

            var section = Ephemera.el('div', 'discover-mentions-section');
            var titleRow = Ephemera.el('div', 'section-title-row');
            var title = Ephemera.el('div', 'section-title', 'Mentions');
            var badge = Ephemera.el('span', 'mentions-badge', String(mentions.length));
            titleRow.appendChild(title);
            titleRow.appendChild(badge);
            section.appendChild(titleRow);

            mentions.slice(0, 5).forEach(function (m) {
                var row = Ephemera.el('div', 'mention-row');
                var author = m.author_handle ? '@' + m.author_handle : 'Someone';
                row.appendChild(Ephemera.el('span', 'mention-author', author));
                row.appendChild(Ephemera.el('span', 'mention-text', ' mentioned you'));
                if (m.body_preview) {
                    var preview = Ephemera.el('div', 'mention-preview');
                    preview.textContent = m.body_preview.length > 80
                        ? m.body_preview.slice(0, 80) + '...'
                        : m.body_preview;
                    row.appendChild(preview);
                }
                if (m.created_at) {
                    var ts = m.created_at;
                    if (ts < 1e12) ts = ts * 1000;
                    row.appendChild(Ephemera.el('span', 'mention-time', Ephemera.timeAgo(ts)));
                }
                section.appendChild(row);
            });

            container.appendChild(section);
        } catch (_e) {
            // Mentions not supported or no mentions -- skip silently
        }
    }

    // ================================================================
    // Browse Groups shortcut
    // ================================================================

    async function renderGroupsShortcut(container) {
        var section = Ephemera.el('div', 'discover-groups-section');

        var titleRow = Ephemera.el('div', 'section-title-row');
        titleRow.appendChild(Ephemera.el('div', 'section-title', 'Groups'));
        var seeAllBtn = Ephemera.el('button', 'btn btn-ghost btn-sm', 'See All');
        seeAllBtn.addEventListener('click', function () { Ephemera.navigate('/groups'); });
        titleRow.appendChild(seeAllBtn);
        section.appendChild(titleRow);

        try {
            var result = await Ephemera.rpc('groups.search', { query: '' });
            var groups = (result.groups || []).slice(0, 3);
            if (groups.length === 0) {
                var hint = Ephemera.el('div', 'discover-groups-hint');
                hint.innerHTML = '<svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M23 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/></svg>';
                hint.appendChild(Ephemera.el('span', '', ' Browse or create groups'));
                hint.addEventListener('click', function () { Ephemera.navigate('/groups'); });
                hint.style.cursor = 'pointer';
                section.appendChild(hint);
            } else {
                var groupsList = Ephemera.el('div', 'discover-groups-list');
                groups.forEach(function (g) {
                    var card = Ephemera.el('div', 'discover-group-card');
                    card.setAttribute('role', 'button');
                    card.setAttribute('tabindex', '0');
                    card.appendChild(Ephemera.el('div', 'discover-group-name', g.name));
                    var meta = (g.member_count || 0) + ' members';
                    if (g.visibility !== 'public') meta += ' | ' + g.visibility;
                    card.appendChild(Ephemera.el('div', 'discover-group-meta', meta));
                    card.addEventListener('click', function () { Ephemera.navigate('/groups'); });
                    groupsList.appendChild(card);
                });
                section.appendChild(groupsList);
            }
        } catch (_e) {
            var hint = Ephemera.el('div', 'discover-groups-hint');
            hint.textContent = 'Browse groups';
            hint.style.cursor = 'pointer';
            hint.addEventListener('click', function () { Ephemera.navigate('/groups'); });
            section.appendChild(hint);
        }

        container.appendChild(section);
    }

    Ephemera.registerRoute('/discover', renderDiscover);

    // Clean up polling when navigating away from discover.
    if (typeof Ephemera.onNavigateAway === 'function') {
        Ephemera.onNavigateAway('/discover', stopDiscoverPolling);
    }
})();
