/**
 * Ephemera -- Connections View
 *
 * Dedicated connections management with:
 * - List of connected users (name, @handle, avatar)
 * - Each connection: Message, Profile, Remove buttons
 * - Pending incoming requests (accept/reject)
 * - Pending outgoing requests (cancel/resend)
 *
 * RPC calls:
 *   - social.list_connections { status }
 *   - social.accept { from }
 *   - social.reject { from }
 *   - social.disconnect { target }
 *   - social.resend_request { target, message }
 *   - social.cancel_request { target }
 */
(function () {
    'use strict';

    var _connectionsPollTimer = null;

    function stopConnectionsPolling() {
        if (_connectionsPollTimer) {
            clearInterval(_connectionsPollTimer);
            _connectionsPollTimer = null;
        }
    }

    function renderConnectionCard(conn, parentContainer, section) {
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

        // Show connection request message for incoming
        if (conn.message && section === 'incoming') {
            var msgEl = Ephemera.el('div', 'connection-mutual');
            msgEl.style.fontStyle = 'italic';
            msgEl.textContent = '"' + conn.message + '"';
            info.appendChild(msgEl);
        }

        // Show time since connected
        if (conn.since && section === 'connected') {
            var ts = conn.since;
            if (ts < 1e12) ts = ts * 1000;
            info.appendChild(Ephemera.el('div', 'connection-since', 'Connected ' + Ephemera.timeAgo(ts)));
        }

        card.appendChild(info);

        // Action buttons based on section
        var actions = Ephemera.el('div', 'connection-actions');

        if (section === 'incoming') {
            var acceptBtn = Ephemera.el('button', 'btn btn-primary btn-sm', 'Accept');
            acceptBtn.setAttribute('aria-label', 'Accept ' + displayName);
            acceptBtn.addEventListener('click', async function () {
                acceptBtn.disabled = true;
                try {
                    await Ephemera.rpc('social.accept', { from: conn.pseudonym_id });
                    Ephemera.showToast('Connected with ' + displayName + '!', 'success');
                    renderConnections(parentContainer);
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

        } else if (section === 'outgoing') {
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
                    renderConnections(parentContainer);
                } catch (err) {
                    Ephemera.showToast('Failed: ' + err.message, 'error');
                    resendBtn.disabled = false;
                    resendBtn.textContent = 'Resend';
                }
            });
            actions.appendChild(resendBtn);

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
            actions.appendChild(cancelBtn);

        } else if (section === 'connected') {
            var msgBtn = Ephemera.el('button', 'btn btn-primary btn-sm', 'Message');
            msgBtn.setAttribute('aria-label', 'Message ' + displayName);
            msgBtn.addEventListener('click', function () {
                Ephemera.store.set({ messageTarget: conn.pseudonym_id });
                Ephemera.navigate('/messages');
            });
            actions.appendChild(msgBtn);

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
            actions.appendChild(removeBtn);
        }

        card.appendChild(actions);
        return card;
    }

    async function renderConnections(container) {
        stopConnectionsPolling();
        container.innerHTML = '';

        // Header
        var header = Ephemera.el('div', 'discover-header');
        header.appendChild(Ephemera.el('h1', '', 'Connections'));
        container.appendChild(header);

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
            var incoming = [];
            var outgoing = [];
            var active = [];

            if (Array.isArray(connections)) {
                connections.forEach(function (c) {
                    if (c.status === 'pending_incoming') incoming.push(c);
                    else if (c.status === 'pending_outgoing') outgoing.push(c);
                    else if (c.status !== 'blocked') active.push(c);
                });
            }

            // Pending incoming requests
            if (incoming.length > 0) {
                var incomingBanner = Ephemera.el('div', 'pending-banner');
                incomingBanner.innerHTML = '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2"><path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><line x1="19" y1="8" x2="19" y2="14"/><line x1="22" y1="11" x2="16" y2="11"/></svg>';
                incomingBanner.appendChild(document.createTextNode(
                    ' ' + incoming.length + ' incoming request' +
                    (incoming.length > 1 ? 's' : '')));
                container.appendChild(incomingBanner);

                container.appendChild(Ephemera.el('div', 'section-title',
                    'Incoming Requests (' + incoming.length + ')'));
                var inList = Ephemera.el('div', '');
                inList.setAttribute('role', 'list');
                incoming.forEach(function (c) {
                    inList.appendChild(renderConnectionCard(c, container, 'incoming'));
                });
                container.appendChild(inList);
            }

            // Pending outgoing requests
            if (outgoing.length > 0) {
                container.appendChild(Ephemera.el('div', 'section-title',
                    'Sent Requests (' + outgoing.length + ')'));
                var outList = Ephemera.el('div', '');
                outList.setAttribute('role', 'list');
                outgoing.forEach(function (c) {
                    outList.appendChild(renderConnectionCard(c, container, 'outgoing'));
                });
                container.appendChild(outList);
            }

            // Active connections
            if (active.length > 0) {
                container.appendChild(Ephemera.el('div', 'section-title',
                    'Connected (' + active.length + ')'));
                var activeList = Ephemera.el('div', '');
                activeList.setAttribute('role', 'list');
                active.forEach(function (c) {
                    activeList.appendChild(renderConnectionCard(c, container, 'connected'));
                });
                container.appendChild(activeList);
            }

            // Groups link at the bottom of the connections list
            var groupsSection = Ephemera.el('div', 'groups-link-section');
            groupsSection.style.marginTop = '1.5rem';
            groupsSection.style.paddingTop = '1rem';
            groupsSection.style.borderTop = '1px solid var(--border, rgba(255,255,255,0.08))';
            var groupsBtn = Ephemera.el('button', 'btn btn-ghost', 'View Groups');
            groupsBtn.style.width = '100%';
            groupsBtn.innerHTML =
                '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" style="vertical-align:middle;margin-right:8px;">' +
                '<path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/>' +
                '<path d="M23 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/></svg>' +
                '<span style="vertical-align:middle;">Groups</span>';
            groupsBtn.addEventListener('click', function () { Ephemera.navigate('/groups'); });
            groupsSection.appendChild(groupsBtn);
            container.appendChild(groupsSection);

            // Empty state
            if (incoming.length === 0 && outgoing.length === 0 && active.length === 0) {
                var empty = Ephemera.el('div', 'empty-state');
                var icon = Ephemera.el('div', 'empty-state-icon');
                icon.innerHTML = '<svg viewBox="0 0 24 24" width="64" height="64" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M23 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/></svg>';
                empty.appendChild(icon);
                empty.appendChild(Ephemera.el('h3', '', 'No connections yet'));
                empty.appendChild(Ephemera.el('p', '',
                    'Go to Discover to find people and connect.'));
                var discoverBtn = Ephemera.el('button', 'btn btn-primary', 'Go to Discover');
                discoverBtn.addEventListener('click', function () { Ephemera.navigate('/discover'); });
                empty.appendChild(discoverBtn);
                container.appendChild(empty);
            }

            // Poll for changes every 10 seconds
            var lastSnapshot = JSON.stringify(connections);
            _connectionsPollTimer = setInterval(async function () {
                if (!container.parentNode) {
                    stopConnectionsPolling();
                    return;
                }
                try {
                    var freshResult = await Ephemera.rpc('social.list_connections', { status: 'all' });
                    var freshConns = freshResult.connections || freshResult || [];
                    var freshSnapshot = JSON.stringify(freshConns);
                    if (freshSnapshot !== lastSnapshot) {
                        lastSnapshot = freshSnapshot;
                        renderConnections(container);
                    }
                } catch (_e) {
                    // Silently ignore poll failures
                }
            }, 10000);

        } catch (err) {
            container.removeChild(loading);
            var empty = Ephemera.el('div', 'empty-state');
            empty.appendChild(Ephemera.el('h3', '', 'Could not load connections'));
            empty.appendChild(Ephemera.el('p', '', err.message || 'Please try again.'));
            container.appendChild(empty);
        }
    }

    Ephemera.registerRoute('/connections', renderConnections);

    // Clean up polling when navigating away.
    if (typeof Ephemera.onNavigateAway === 'function') {
        Ephemera.onNavigateAway('/connections', stopConnectionsPolling);
    }
})();
