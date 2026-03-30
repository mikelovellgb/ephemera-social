/**
 * Ephemera -- Main application module
 *
 * Provides routing, RPC client, state management, view lifecycle,
 * and page transitions. Uses vanilla JS with a simple reactive store.
 */

// eslint-disable-next-line no-var
var Ephemera = (function () {
    'use strict';

    // ================================================================
    // Reactive Store
    // ================================================================

    function createStore(initial) {
        var state = Object.assign({}, initial);
        var listeners = new Set();

        return {
            get: function () { return state; },
            set: function (updates) {
                state = Object.assign({}, state, updates);
                listeners.forEach(function (fn) { fn(state); });
            },
            subscribe: function (fn) {
                listeners.add(fn);
                return function () { listeners.delete(fn); };
            },
        };
    }

    var store = createStore({
        identity: null,
        hasIdentity: false,
        hasKeystore: false,
        route: '',
        prevRoute: '',
        loading: true,
        nodeStatus: null,
        replyTo: null,       // { content_hash, author_handle, author_name, body_preview }
        composeAudience: 'everyone',
    });

    // ================================================================
    // RPC Client
    // ================================================================

    var rpcIdCounter = 0;

    async function rpc(method, params) {
        params = params || {};
        var id = ++rpcIdCounter;

        var request = {
            jsonrpc: '2.0',
            method: method,
            params: params,
            id: id,
        };

        var response;

        if (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke) {
            response = await window.__TAURI_INTERNALS__.invoke('rpc', { request: request });
        } else {
            var headers = { 'Content-Type': 'application/json' };
            if (window.__EPHEMERA_RPC_TOKEN__) {
                headers['Authorization'] = 'Bearer ' + window.__EPHEMERA_RPC_TOKEN__;
            }

            var res = await fetch('/rpc', {
                method: 'POST',
                headers: headers,
                body: JSON.stringify(request),
            });

            if (!res.ok) {
                throw new Error('HTTP ' + res.status + ': ' + res.statusText);
            }

            response = await res.json();
        }

        if (response.error) {
            var err = new Error(response.error.message || 'RPC error');
            err.code = response.error.code;
            err.data = response.error.data;
            throw err;
        }

        return response.result;
    }

    // ================================================================
    // Toast Notifications
    // ================================================================

    function showToast(message, type) {
        type = type || 'info';
        var container = document.querySelector('.toast-container');
        if (!container) {
            container = document.createElement('div');
            container.className = 'toast-container';
            document.body.appendChild(container);
        }

        var toast = document.createElement('div');
        toast.className = 'toast toast-' + type;
        toast.setAttribute('role', 'alert');
        toast.setAttribute('aria-live', 'polite');
        toast.textContent = message;
        container.appendChild(toast);

        // Trigger entrance animation
        requestAnimationFrame(function () {
            toast.classList.add('toast-visible');
        });

        setTimeout(function () {
            toast.classList.remove('toast-visible');
            toast.classList.add('toast-exit');
            setTimeout(function () { toast.remove(); }, 300);
        }, 3500);
    }

    // ================================================================
    // Router with Page Transitions
    // ================================================================

    var routes = {};
    var routeOrder = ['/feed', '/discover', '/connections', '/compose', '/groups', '/messages', '/profile'];

    function registerRoute(path, renderFn) {
        routes[path] = renderFn;
    }

    function navigate(route) {
        window.location.hash = route;
    }

    function getCurrentRoute() {
        var hash = window.location.hash || '';
        return hash.replace(/^#/, '') || '/onboarding';
    }

    function getTransitionClass(from, to) {
        if (from === '/onboarding' || to === '/onboarding') return 'view-enter';
        if (from === '/unlock' || to === '/unlock') return 'view-enter';
        if (to === '/compose') return 'view-slide-up';
        if (from === '/compose') return 'view-enter';
        var fromIdx = routeOrder.indexOf(from);
        var toIdx = routeOrder.indexOf(to);
        if (fromIdx === -1 || toIdx === -1) return 'view-enter';
        return toIdx > fromIdx ? 'view-slide-left' : 'view-slide-right';
    }

    function updateNavHighlight(route) {
        document.querySelectorAll('.nav-item').forEach(function (item) {
            var itemRoute = item.getAttribute('data-route');
            if (route.indexOf(itemRoute) !== -1) {
                item.classList.add('active');
            } else {
                item.classList.remove('active');
            }
        });
        // Compose button active state
        var composeBtn = document.getElementById('nav-compose-btn');
        if (composeBtn) {
            if (route === '/compose') {
                composeBtn.classList.add('active');
            } else {
                composeBtn.classList.remove('active');
            }
        }
    }

    async function handleRouteChange() {
        var route = getCurrentRoute();
        var state = store.get();
        var prevRoute = state.route || '';

        // If not onboarded, force onboarding or unlock
        if (!state.hasIdentity && route !== '/onboarding' && route !== '/unlock') {
            if (state.hasKeystore) {
                navigate('/unlock');
            } else {
                navigate('/onboarding');
            }
            return;
        }

        // Show/hide nav bar
        var navBar = document.getElementById('nav-bar');
        if (navBar) {
            if (route === '/onboarding' || route === '/unlock') {
                navBar.classList.add('hidden');
            } else {
                navBar.classList.remove('hidden');
            }
        }

        updateNavHighlight(route);
        store.set({ route: route, prevRoute: prevRoute });

        var mainContent = document.getElementById('main-content');
        if (!mainContent) return;

        // Find matching route handler
        var handler = routes[route];
        if (handler) {
            mainContent.innerHTML = '';

            // Apply page transition animation
            var transClass = getTransitionClass(prevRoute, route);
            mainContent.classList.remove('view-enter', 'view-slide-left', 'view-slide-right', 'view-slide-up');
            // Force reflow so the animation replays
            void mainContent.offsetWidth;
            mainContent.classList.add(transClass);

            try {
                await handler(mainContent);
            } catch (err) {
                console.error('Route render error:', err);
                mainContent.innerHTML =
                    '<div class="empty-state">' +
                    '<div class="empty-state-icon">' +
                    '<svg viewBox="0 0 24 24" width="48" height="48" fill="none" stroke="currentColor" stroke-width="1.5"><circle cx="12" cy="12" r="10"/><line x1="15" y1="9" x2="9" y2="15"/><line x1="9" y1="9" x2="15" y2="15"/></svg>' +
                    '</div>' +
                    '<h3>Something went wrong</h3>' +
                    '<p>Please try again.</p>' +
                    '</div>';
            }
        } else {
            navigate(state.hasIdentity ? '/feed' : '/onboarding');
        }
    }

    // ================================================================
    // Utility Helpers
    // ================================================================

    function timeAgo(timestampMs) {
        var now = Date.now();
        var diff = now - timestampMs;

        if (diff < 60000) return 'now';
        if (diff < 3600000) return Math.floor(diff / 60000) + 'm';
        if (diff < 86400000) return Math.floor(diff / 3600000) + 'h';
        if (diff < 604800000) return Math.floor(diff / 86400000) + 'd';
        return new Date(timestampMs).toLocaleDateString();
    }

    function formatTTL(remainingSecs) {
        if (remainingSecs <= 0) return 'expired';
        if (remainingSecs < 3600) return Math.ceil(remainingSecs / 60) + 'm left';
        if (remainingSecs < 86400) return Math.ceil(remainingSecs / 3600) + 'h left';
        return Math.ceil(remainingSecs / 86400) + 'd left';
    }

    function ttlPercent(remainingSecs, totalTtlSecs) {
        if (totalTtlSecs <= 0) return 1;
        return Math.max(0, Math.min(1, remainingSecs / totalTtlSecs));
    }

    function getInitials(name) {
        if (!name) return '?';
        return name.split(/\s+/).map(function (w) { return w[0]; }).join('').toUpperCase().slice(0, 2);
    }

    function getDisplayHandle(identity) {
        if (!identity) return null;
        if (identity.handle) {
            // Handle may already include @ prefix from backend
            var h = identity.handle;
            return h.charAt(0) === '@' ? h : '@' + h;
        }
        // Fall through to show a truncated public key identifier
        var id = identity.pseudonym_id || identity.public_key || identity.pubkey || '';
        if (id) {
            return id.length > 16 ? '@' + id.slice(0, 6) + '...' + id.slice(-4) : '@' + id;
        }
        return null;
    }

    /** Create an element with optional class and inner text. */
    function el(tag, className, text) {
        var e = document.createElement(tag);
        if (className) e.className = className;
        if (text) e.textContent = text;
        return e;
    }

    /** Create an avatar element with initials and optional size class.
     *  If avatarUrl is provided, render an image instead of initials. */
    function avatar(name, sizeClass, avatarUrl) {
        var div = el('div', 'avatar' + (sizeClass ? ' ' + sizeClass : ''));
        div.setAttribute('aria-hidden', 'true');
        if (avatarUrl) {
            var img = document.createElement('img');
            img.src = avatarUrl;
            img.alt = name || '';
            img.className = 'avatar-img';
            img.draggable = false;
            img.onerror = function () {
                // Fall back to initials on load failure
                img.remove();
                div.textContent = getInitials(name);
            };
            div.appendChild(img);
        } else {
            div.textContent = getInitials(name);
        }
        return div;
    }

    /** Create an avatar with a TTL ring wrapper. */
    function avatarWithRing(name, ttlPct, sizeClass, avatarUrl) {
        var ring = el('div', 'avatar-ring');
        ring.style.setProperty('--ring-pct', ttlPct.toFixed(2));
        if (ttlPct > 0.5) ring.classList.add('fresh');
        else if (ttlPct > 0.15) ring.classList.add('aging');
        else ring.classList.add('critical');
        ring.appendChild(avatar(name, sizeClass, avatarUrl));
        return ring;
    }

    /** Create skeleton loading placeholder posts. */
    function skeletonPosts(count) {
        var frag = document.createDocumentFragment();
        for (var i = 0; i < count; i++) {
            var sk = el('div', 'skeleton-post');
            sk.appendChild(el('div', 'skeleton skeleton-avatar'));
            var body = el('div', 'skeleton-post-body');
            body.appendChild(el('div', 'skeleton skeleton-line short'));
            body.appendChild(el('div', 'skeleton skeleton-line long'));
            body.appendChild(el('div', 'skeleton skeleton-line medium'));
            sk.appendChild(body);
            frag.appendChild(sk);
        }
        return frag;
    }

    /** Format an audience type into a display label. */
    function audienceLabel(audience) {
        if (!audience || audience === 'everyone') return 'Public';
        if (audience === 'connections') return 'Connections';
        if (audience.startsWith('topic:')) return '#' + audience.slice(6);
        return audience;
    }

    /** Highlight @mentions in text, returning safe HTML. */
    function highlightMentions(text) {
        var escaped = text
            .replace(/&/g, '&amp;')
            .replace(/</g, '&lt;')
            .replace(/>/g, '&gt;')
            .replace(/"/g, '&quot;');
        return escaped.replace(/(^|\s)@([a-zA-Z0-9_-]{1,30})/g,
            '$1<span class="mention-highlight">@$2</span>');
    }

    /** Get audience icon SVG string. */
    function audienceIcon(audience) {
        if (audience === 'connections') {
            return '<svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2"><path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/></svg>';
        }
        if (audience && audience.startsWith('topic:')) {
            return '<svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2"><line x1="4" y1="9" x2="20" y2="9"/><line x1="4" y1="15" x2="20" y2="15"/><line x1="10" y1="3" x2="8" y2="21"/><line x1="16" y1="3" x2="14" y2="21"/></svg>';
        }
        return '<svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"/><line x1="2" y1="12" x2="22" y2="12"/><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/></svg>';
    }

    // ================================================================
    // Notification Badge
    // ================================================================

    var _notificationPollTimer = null;

    function updateNotificationBadge(count) {
        var badge = document.getElementById('notification-badge');
        if (!badge) return;
        if (count > 0) {
            badge.textContent = count > 99 ? '99+' : String(count);
            badge.style.display = '';
        } else {
            badge.style.display = 'none';
        }
    }

    async function pollNotificationCount() {
        try {
            var result = await rpc('notifications.count', {});
            updateNotificationBadge(result.unread_count || 0);
        } catch (_e) {
            // Silently ignore (identity locked, not yet initialized, etc.)
        }
    }

    function startNotificationPolling() {
        if (_notificationPollTimer) return;
        // Poll every 15 seconds
        pollNotificationCount();
        _notificationPollTimer = setInterval(pollNotificationCount, 15000);
    }

    // ================================================================
    // Initialization
    // ================================================================

    async function init() {
        window.addEventListener('hashchange', handleRouteChange);

        // Compose button in nav
        var composeBtn = document.getElementById('nav-compose-btn');
        if (composeBtn) {
            composeBtn.addEventListener('click', function () {
                store.set({ replyTo: null });
                navigate('/compose');
            });
        }

        try {
            var profile = await rpc('identity.get_active');
            if (profile && (profile.pubkey || profile.public_key)) {
                store.set({
                    identity: profile,
                    hasIdentity: true,
                    loading: false,
                });
                var route = getCurrentRoute();
                if (route === '/onboarding' || route === '/' || route === '/unlock') {
                    navigate('/feed');
                    return;
                }
            } else {
                store.set({ loading: false });
            }
        } catch (_e) {
            // get_active failed -- check if a keystore exists (locked state)
            try {
                var ks = await rpc('identity.has_keystore');
                if (ks && ks.exists) {
                    // Keystore exists but locked -- show unlock screen
                    store.set({ loading: false, hasKeystore: true });
                    navigate('/unlock');
                    handleRouteChange();
                    return;
                }
            } catch (_e2) { /* no keystore */ }
            store.set({ loading: false });
        }

        handleRouteChange();

        // Handle ephemera:// deep links (from QR codes scanned by system camera)
        handleDeepLink();

        // Start notification badge polling
        startNotificationPolling();
    }

    function handleDeepLink() {
        // Check URL for deep link data (Tauri passes it as a query param or hash)
        var url = window.location.href;
        var match = url.match(/ephemera:\/\/connect\/([a-fA-F0-9]+)/);
        if (!match) {
            // Also check for a query parameter (some WebViews pass deep link data this way)
            var params = new URLSearchParams(window.location.search);
            var deepLink = params.get('deeplink') || params.get('url');
            if (deepLink) {
                match = deepLink.match(/ephemera:\/\/connect\/([a-fA-F0-9]+)/);
            }
        }
        if (match && match[1]) {
            var pubkey = match[1];
            // Wait for identity to be ready, then establish network + social connection
            setTimeout(async function () {
                var state = store.get();
                if (!state.hasIdentity) return;
                try {
                    // Step 1: Establish network-level connection via Iroh.
                    // The pubkey IS the Iroh NodeId, so Iroh discovery will find the peer.
                    try {
                        await rpc('network.connect', { node_id: pubkey });
                    } catch (e) {
                        // Network connection may fail if peer is offline -- that's OK,
                        // the social request will be queued and delivered later.
                        console.warn('Deep link: network connect failed (peer may be offline):', e.message || e);
                    }

                    // Step 2: Send social connection request.
                    await rpc('social.connect', { target: pubkey, message: 'Connected via QR code!' });
                    showToast('Connection request sent!', 'success');
                    navigate('/discover');
                } catch (err) {
                    showToast('Connection failed: ' + err.message, 'error');
                }
            }, 1000);
        }
    }

    // ================================================================
    // Public API
    // ================================================================

    return {
        store: store,
        rpc: rpc,
        navigate: navigate,
        registerRoute: registerRoute,
        getCurrentRoute: getCurrentRoute,
        showToast: showToast,
        timeAgo: timeAgo,
        formatTTL: formatTTL,
        ttlPercent: ttlPercent,
        getInitials: getInitials,
        getDisplayHandle: getDisplayHandle,
        el: el,
        avatar: avatar,
        avatarWithRing: avatarWithRing,
        skeletonPosts: skeletonPosts,
        audienceLabel: audienceLabel,
        audienceIcon: audienceIcon,
        highlightMentions: highlightMentions,
        updateNotificationBadge: updateNotificationBadge,
        init: init,
    };
})();
