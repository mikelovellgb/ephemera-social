/**
 * Ephemera -- Feed View
 *
 * Modern feed with:
 * - Skeleton loading shimmer
 * - Infinite scroll via IntersectionObserver
 * - Avatar TTL rings
 * - @handle display + audience badges + reply count + time remaining
 * - Reply flow that passes context to compose
 * - Animated reaction picker
 * - Rich empty state
 *
 * RPC calls:
 *   - feed.connections { limit, after }
 *   - social.react { content_hash, emoji, action }
 *   - moderation.report { content_hash, reason }
 *   - posts.delete { content_hash }
 */
(function () {
    'use strict';

    var highlightMentions = Ephemera.highlightMentions;

    var REACTION_EMOJIS = [
        { key: 'heart',    label: 'Heart',    icon: '\u2665' },
        { key: 'laugh',    label: 'Laugh',    icon: '\uD83D\uDE04' },
        { key: 'fire',     label: 'Fire',     icon: '\uD83D\uDD25' },
        { key: 'sad',      label: 'Sad',      icon: '\uD83D\uDE22' },
        { key: 'thinking', label: 'Thinking', icon: '\uD83E\uDD14' },
    ];

    var REPORT_REASONS = [
        { key: 'spam', label: 'Spam' },
        { key: 'harassment', label: 'Harassment' },
        { key: 'hate_speech', label: 'Hate speech' },
        { key: 'violence', label: 'Violence' },
        { key: 'csam', label: 'Child exploitation' },
        { key: 'other', label: 'Other' },
    ];

    var feedCursor = null;
    var isLoadingMore = false;
    var observer = null;

    function renderPostCard(post) {
        var card = Ephemera.el('article', 'post-card');
        card.setAttribute('aria-label', 'Post by ' + (post.author_display_name || 'Anonymous'));

        // TTL calculations
        var remainingSecs = post.remaining_seconds != null
            ? post.remaining_seconds
            : ((post.expires_at || 0) - Date.now()) / 1000;
        var totalTtl = post.ttl_seconds || 2592000;
        var pct = Ephemera.ttlPercent(remainingSecs, totalTtl);
        card.style.setProperty('--ttl-pct', pct.toFixed(2));
        card.setAttribute('data-ttl-pct', '');

        if (pct < 0.15) card.classList.add('expiring-soon');

        // Reply context
        if (post.is_reply && post.parent_author_handle) {
            var replyCtx = Ephemera.el('div', 'post-reply-context');
            replyCtx.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="15 18 9 12 15 6"/></svg>';
            replyCtx.appendChild(document.createTextNode('Replying to @' + post.parent_author_handle));
            card.appendChild(replyCtx);
        }

        // Sensitivity label
        if (post.sensitivity) {
            var sensLabel = Ephemera.el('div', 'sensitivity-label', post.sensitivity.toUpperCase());
            if (post.sensitivity === 'spoiler') sensLabel.classList.add('spoiler');
            card.appendChild(sensLabel);
        }

        // Header: avatar ring + author row + meta
        var header = Ephemera.el('div', 'post-header');

        var displayName = post.author_display_name || 'Anonymous';
        // For own posts, prefer the local identity avatar (most up-to-date)
        var avatarUrl = post.author_avatar_url || null;
        if (post.is_own) {
            var identity = Ephemera.store.get().identity;
            if (identity && identity.avatar_url) avatarUrl = identity.avatar_url;
        }
        var avatarEl = Ephemera.avatarWithRing(displayName, pct, null, avatarUrl);
        header.appendChild(avatarEl);

        var authorInfo = Ephemera.el('div', 'post-author-info');

        var authorRow = Ephemera.el('div', 'post-author-row');
        authorRow.appendChild(Ephemera.el('span', 'post-author-name', displayName));

        // Show @handle
        var handleText = post.author_handle
            ? '@' + post.author_handle
            : (post.author
                ? (post.author.length > 16
                    ? '@' + post.author.slice(0, 6) + '...' + post.author.slice(-4)
                    : '@' + post.author)
                : null);
        if (handleText) {
            authorRow.appendChild(Ephemera.el('span', 'post-author-handle', handleText));
        }

        var createdAt = post.created_at || Date.now();
        authorRow.appendChild(Ephemera.el('span', 'post-timestamp', Ephemera.timeAgo(createdAt)));

        authorInfo.appendChild(authorRow);

        // Meta row: audience badge + time remaining
        var metaRow = Ephemera.el('div', 'post-meta-row');

        // Audience badge
        var audience = post.audience || 'everyone';
        var badge = Ephemera.el('span', 'audience-badge');
        badge.innerHTML = Ephemera.audienceIcon(audience) + ' ' + Ephemera.audienceLabel(audience);
        metaRow.appendChild(badge);

        if (remainingSecs > 0) {
            var expiryEl = Ephemera.el('div', 'post-expiry');
            expiryEl.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/></svg>';
            expiryEl.appendChild(document.createTextNode(' ' + Ephemera.formatTTL(remainingSecs)));
            expiryEl.setAttribute('aria-label', 'Expires in ' + Ephemera.formatTTL(remainingSecs));
            metaRow.appendChild(expiryEl);
        }

        authorInfo.appendChild(metaRow);
        header.appendChild(authorInfo);
        card.appendChild(header);

        // Body (with mention highlighting)
        if (post.body_html) {
            var bodyHtml = Ephemera.el('div', 'post-body');
            bodyHtml.innerHTML = post.body_html;
            card.appendChild(bodyHtml);
        } else if (post.body) {
            var bodyEl = Ephemera.el('div', 'post-body');
            bodyEl.innerHTML = highlightMentions(post.body);
            card.appendChild(bodyEl);
        }

        // Media
        if (post.media && Array.isArray(post.media) && post.media.length > 0) {
            post.media.forEach(function (m) {
                var mediaWrap = Ephemera.el('div', 'post-media');
                if (m.media_type === 'video') {
                    var video = document.createElement('video');
                    video.controls = true;
                    video.preload = 'metadata';
                    video.style.width = '100%';
                    if (m.variants && m.variants[0]) video.src = m.variants[0].url;
                    mediaWrap.appendChild(video);
                } else {
                    var img = document.createElement('img');
                    img.alt = m.alt_text || 'Post media';
                    img.loading = 'lazy';
                    if (m.variants && m.variants[0]) img.src = m.variants[0].url;
                    else if (post.media_url) img.src = post.media_url;
                    mediaWrap.appendChild(img);
                }
                card.appendChild(mediaWrap);
            });
        } else if (post.media_url) {
            var mediaWrap = Ephemera.el('div', 'post-media');
            var img = document.createElement('img');
            img.src = post.media_url;
            img.alt = 'Post media';
            img.loading = 'lazy';
            mediaWrap.appendChild(img);
            card.appendChild(mediaWrap);
        }

        // TTL progress bar
        var ttlBar = Ephemera.el('div', 'ttl-bar');
        var ttlFill = Ephemera.el('div', 'ttl-bar-fill');
        ttlFill.style.width = (pct * 100).toFixed(1) + '%';
        if (pct > 0.5) ttlFill.classList.add('fresh');
        else if (pct > 0.1) ttlFill.classList.add('aging');
        else ttlFill.classList.add('critical');
        ttlBar.appendChild(ttlFill);
        card.appendChild(ttlBar);

        // Reaction badges (visible to author)
        if (post.reactions && post.is_own) {
            var hasReactions = false;
            var reactionsDiv = Ephemera.el('div', 'post-reactions');
            REACTION_EMOJIS.forEach(function (r) {
                var count = post.reactions[r.key] || 0;
                if (count > 0) {
                    hasReactions = true;
                    var reactionBadge = Ephemera.el('span', 'post-reaction-badge', r.icon + ' ' + count);
                    reactionBadge.setAttribute('aria-label', count + ' ' + r.label);
                    reactionsDiv.appendChild(reactionBadge);
                }
            });
            if (hasReactions) card.appendChild(reactionsDiv);
        }

        // Actions bar
        var actions = Ephemera.el('div', 'post-actions');

        // React
        var reactBtn = Ephemera.el('button', 'post-action-btn');
        var reactIcon = post.my_reaction
            ? (REACTION_EMOJIS.find(function (r) { return r.key === post.my_reaction; }) || {}).icon || '\u2665'
            : '';
        reactBtn.innerHTML = reactIcon
            ? '<span>' + reactIcon + '</span>'
            : '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2"><path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z"/></svg>';
        if (post.my_reaction) reactBtn.classList.add('active');
        reactBtn.setAttribute('aria-label', 'React');
        reactBtn.addEventListener('click', function () { showReactionPicker(card, post); });
        actions.appendChild(reactBtn);

        // Reply (with count)
        var replyBtn = Ephemera.el('button', 'post-action-btn');
        replyBtn.innerHTML = '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2"><path d="M21 11.5a8.38 8.38 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.38 8.38 0 0 1-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.38 8.38 0 0 1 3.8-.9h.5a8.48 8.48 0 0 1 8 8v.5z"/></svg>';
        if (post.reply_count) {
            replyBtn.innerHTML += '<span>' + post.reply_count + '</span>';
        }
        replyBtn.setAttribute('aria-label', 'Reply' + (post.reply_count ? ' (' + post.reply_count + ')' : ''));
        replyBtn.addEventListener('click', function () {
            // Set reply context in store and navigate to compose
            Ephemera.store.set({
                replyTo: {
                    content_hash: post.content_hash,
                    author_handle: post.author_handle || (post.author ? post.author.slice(0, 8) : 'anon'),
                    author_name: post.author_display_name || 'Anonymous',
                    body_preview: (post.body || '').slice(0, 120),
                }
            });
            Ephemera.navigate('/compose');
        });
        actions.appendChild(replyBtn);

        // Report
        var reportBtn = Ephemera.el('button', 'post-action-btn');
        reportBtn.innerHTML = '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2"><path d="M4 15s1-1 4-1 5 2 8 2 4-1 4-1V3s-1 1-4 1-5-2-8-2-4 1-4 1z"/><line x1="4" y1="22" x2="4" y2="15"/></svg>';
        reportBtn.setAttribute('aria-label', 'Report');
        reportBtn.addEventListener('click', function () { showReportDialog(post); });
        actions.appendChild(reportBtn);

        // Delete (own posts)
        if (post.is_own) {
            var deleteBtn = Ephemera.el('button', 'post-action-btn');
            deleteBtn.innerHTML = '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>';
            deleteBtn.setAttribute('aria-label', 'Delete');
            deleteBtn.addEventListener('click', async function () {
                if (!confirm('Delete this post?')) return;
                try {
                    await Ephemera.rpc('posts.delete', { content_hash: post.content_hash });
                    card.style.opacity = '0';
                    card.style.transform = 'scale(0.97)';
                    card.style.transition = 'all 300ms ease-out';
                    setTimeout(function () { card.remove(); }, 300);
                    Ephemera.showToast('Post deleted', 'success');
                } catch (err) {
                    Ephemera.showToast('Delete failed: ' + err.message, 'error');
                }
            });
            actions.appendChild(deleteBtn);
        }

        card.appendChild(actions);
        return card;
    }

    function showReactionPicker(card, post) {
        var existing = card.querySelector('.reaction-picker');
        if (existing) { existing.remove(); return; }

        var picker = Ephemera.el('div', 'reaction-picker');

        REACTION_EMOJIS.forEach(function (r) {
            var btn = Ephemera.el('button', 'post-action-btn', r.icon);
            btn.setAttribute('aria-label', 'React with ' + r.label);
            if (post.my_reaction === r.key) btn.classList.add('active');
            btn.addEventListener('click', async function () {
                var action = post.my_reaction === r.key ? 'remove' : 'add';
                try {
                    await Ephemera.rpc('social.react', {
                        content_hash: post.content_hash,
                        emoji: r.key,
                        action: action,
                    });
                    post.my_reaction = action === 'add' ? r.key : null;
                    picker.remove();
                    Ephemera.showToast(action === 'add' ? 'Reacted!' : 'Removed', 'success');
                } catch (err) {
                    Ephemera.showToast('Failed: ' + err.message, 'error');
                }
            });
            picker.appendChild(btn);
        });

        var actionsBar = card.querySelector('.post-actions');
        if (actionsBar && actionsBar.nextSibling) {
            card.insertBefore(picker, actionsBar.nextSibling);
        } else {
            card.appendChild(picker);
        }
    }

    function showReportDialog(post) {
        var overlay = Ephemera.el('div', 'modal-overlay');
        overlay.setAttribute('role', 'dialog');
        overlay.setAttribute('aria-modal', 'true');
        overlay.setAttribute('aria-label', 'Report post');

        var modal = Ephemera.el('div', 'modal-content');
        modal.appendChild(Ephemera.el('h2', '', 'Report Post'));
        modal.appendChild(Ephemera.el('p', '', 'Why are you reporting this content?'));

        var selectedReason = '';
        var reasons = Ephemera.el('div', 'report-reasons');

        REPORT_REASONS.forEach(function (r) {
            var btn = Ephemera.el('button', 'report-reason', r.label);
            btn.addEventListener('click', function () {
                reasons.querySelectorAll('.report-reason').forEach(function (b) {
                    b.classList.remove('selected');
                });
                btn.classList.add('selected');
                selectedReason = r.key;
            });
            reasons.appendChild(btn);
        });
        modal.appendChild(reasons);

        var actionsRow = Ephemera.el('div', 'modal-actions');

        var cancelBtn = Ephemera.el('button', 'btn btn-ghost', 'Cancel');
        cancelBtn.addEventListener('click', function () { overlay.remove(); });
        actionsRow.appendChild(cancelBtn);

        var submitBtn = Ephemera.el('button', 'btn btn-primary', 'Submit');
        submitBtn.addEventListener('click', async function () {
            if (!selectedReason) {
                Ephemera.showToast('Select a reason', 'error');
                return;
            }
            submitBtn.disabled = true;
            submitBtn.textContent = 'Submitting...';
            try {
                await Ephemera.rpc('moderation.report', {
                    content_hash: post.content_hash,
                    reason: selectedReason,
                });
                Ephemera.showToast('Report submitted', 'success');
                overlay.remove();
            } catch (err) {
                Ephemera.showToast('Failed: ' + err.message, 'error');
                submitBtn.disabled = false;
                submitBtn.textContent = 'Submit';
            }
        });
        actionsRow.appendChild(submitBtn);

        modal.appendChild(actionsRow);
        overlay.appendChild(modal);

        overlay.addEventListener('click', function (e) {
            if (e.target === overlay) overlay.remove();
        });

        function onEsc(e) {
            if (e.key === 'Escape') {
                overlay.remove();
                document.removeEventListener('keydown', onEsc);
            }
        }
        document.addEventListener('keydown', onEsc);

        document.body.appendChild(overlay);
    }

    function renderEmptyFeed(container) {
        var empty = Ephemera.el('div', 'empty-state');
        empty.setAttribute('role', 'status');

        var icon = Ephemera.el('div', 'empty-state-icon');
        icon.innerHTML = '<svg viewBox="0 0 24 24" width="64" height="64" fill="none" stroke="currentColor" stroke-width="1.2"><circle cx="12" cy="12" r="10"/><path d="M8 14s1.5 2 4 2 4-2 4-2"/><line x1="9" y1="9" x2="9.01" y2="9"/><line x1="15" y1="9" x2="15.01" y2="9"/></svg>';
        empty.appendChild(icon);

        empty.appendChild(Ephemera.el('h3', '', 'Welcome to Ephemera!'));
        empty.appendChild(Ephemera.el('p', '',
            'Make your first post or find people to follow. Everything here is fleeting -- speak freely.'));

        var row = Ephemera.el('div', '');
        row.style.cssText = 'display:flex;gap:12px;justify-content:center;flex-wrap:wrap;';

        var postBtn = Ephemera.el('button', 'btn btn-primary', 'Create a Post');
        postBtn.addEventListener('click', function () {
            Ephemera.store.set({ replyTo: null });
            Ephemera.navigate('/compose');
        });
        row.appendChild(postBtn);

        var discoverBtn = Ephemera.el('button', 'btn btn-secondary', 'Find People');
        discoverBtn.addEventListener('click', function () { Ephemera.navigate('/discover'); });
        row.appendChild(discoverBtn);

        empty.appendChild(row);
        container.appendChild(empty);
    }

    function renderCaughtUp(container) {
        var div = Ephemera.el('div', 'feed-caught-up');
        div.setAttribute('role', 'status');
        div.appendChild(Ephemera.el('div', 'caught-up-icon', '\u2714'));
        div.appendChild(Ephemera.el('p', '',
            'You\'re all caught up. Go touch grass.'));
        container.appendChild(div);
    }

    function renderNetworkError(container) {
        var empty = Ephemera.el('div', 'empty-state');
        empty.setAttribute('role', 'alert');

        var icon = Ephemera.el('div', 'empty-state-icon');
        icon.innerHTML = '<svg viewBox="0 0 24 24" width="64" height="64" fill="none" stroke="currentColor" stroke-width="1.2"><circle cx="12" cy="12" r="10"/><line x1="15" y1="9" x2="9" y2="15"/><line x1="9" y1="9" x2="15" y2="15"/></svg>';
        empty.appendChild(icon);

        empty.appendChild(Ephemera.el('h3', '', 'Could not load feed'));
        empty.appendChild(Ephemera.el('p', '',
            'Check your connection and try again.'));

        var retryBtn = Ephemera.el('button', 'btn btn-secondary', 'Retry');
        retryBtn.addEventListener('click', function () {
            var mainContent = document.getElementById('main-content');
            if (mainContent) renderFeed(mainContent);
        });
        empty.appendChild(retryBtn);

        container.appendChild(empty);
    }

    function normalizePost(raw) {
        var now = Date.now();
        var identity = Ephemera.store.get().identity;
        var myPubkey = identity ? (identity.pubkey || identity.public_key || '') : '';

        var createdAtMs = raw.created_at;
        if (createdAtMs && createdAtMs < 1e12) createdAtMs = createdAtMs * 1000;

        var expiresAtMs = raw.expires_at;
        if (expiresAtMs && expiresAtMs < 1e12) expiresAtMs = expiresAtMs * 1000;

        var remainingSecs = expiresAtMs ? (expiresAtMs - now) / 1000 : 2592000;
        var ttlSecs = raw.ttl_seconds || 2592000;

        var authorHex = (raw.author || '').toLowerCase();
        var isOwn = myPubkey && authorHex === myPubkey.toLowerCase();

        return {
            content_hash: raw.content_hash || '',
            author: raw.author || '',
            author_handle: raw.author_handle || null,
            author_display_name: raw.author_display_name || null,
            author_avatar_url: raw.author_avatar_url || null,
            body: raw.body || raw.body_preview || '',
            body_html: raw.body_html || null,
            created_at: createdAtMs || now,
            expires_at: expiresAtMs || 0,
            remaining_seconds: remainingSecs,
            ttl_seconds: ttlSecs,
            is_own: isOwn,
            is_reply: raw.is_reply || false,
            parent_author_handle: raw.parent_author_handle || null,
            audience: raw.audience || 'everyone',
            media: raw.media || null,
            media_url: raw.media_url || null,
            reactions: raw.reactions || null,
            my_reaction: raw.my_reaction || null,
            reply_count: raw.reply_count || 0,
            sensitivity: raw.sensitivity || null,
            media_count: raw.media_count || 0,
        };
    }

    async function loadMorePosts(container, sentinel) {
        if (isLoadingMore || !feedCursor) return;
        isLoadingMore = true;

        try {
            var more = await Ephemera.rpc('feed.connections', {
                limit: 50,
                after: feedCursor,
            });
            var morePosts = (more.posts || more || []).map(normalizePost);
            feedCursor = more.cursor || more.next_cursor || null;

            morePosts.forEach(function (post) {
                container.insertBefore(renderPostCard(post), sentinel);
            });

            if (more.is_caught_up || !more.has_more || !feedCursor) {
                if (observer) observer.disconnect();
                sentinel.remove();
                renderCaughtUp(container);
            }
        } catch (err) {
            Ephemera.showToast('Failed to load more: ' + err.message, 'error');
        }

        isLoadingMore = false;
    }

    async function renderFeed(container) {
        container.innerHTML = '';
        feedCursor = null;
        isLoadingMore = false;
        if (observer) { observer.disconnect(); observer = null; }

        // Header
        var header = Ephemera.el('div', 'feed-header');
        header.appendChild(Ephemera.el('h1', '', 'Ephemera'));

        var refreshBtn = Ephemera.el('button', 'btn btn-ghost btn-sm');
        refreshBtn.innerHTML = '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2"><polyline points="23 4 23 10 17 10"/><polyline points="1 20 1 14 7 14"/><path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/></svg>';
        refreshBtn.setAttribute('aria-label', 'Refresh feed');
        refreshBtn.addEventListener('click', function () { renderFeed(container); });
        header.appendChild(refreshBtn);
        container.appendChild(header);

        // Skeleton loading
        container.appendChild(Ephemera.skeletonPosts(4));

        try {
            var result = await Ephemera.rpc('feed.connections', { limit: 50 });

            // Remove skeletons
            container.querySelectorAll('.skeleton-post').forEach(function (sk) { sk.remove(); });

            var posts = result.posts || result || [];

            if (!Array.isArray(posts) || posts.length === 0) {
                renderEmptyFeed(container);
                return;
            }

            posts = posts.map(normalizePost);
            feedCursor = result.cursor || result.next_cursor || null;

            posts.forEach(function (post) {
                container.appendChild(renderPostCard(post));
            });

            // Infinite scroll
            if (result.has_more && feedCursor) {
                var sentinel = Ephemera.el('div', 'scroll-sentinel');
                container.appendChild(sentinel);

                observer = new IntersectionObserver(function (entries) {
                    if (entries[0].isIntersecting) {
                        loadMorePosts(container, sentinel);
                    }
                }, { rootMargin: '200px' });

                observer.observe(sentinel);
            } else {
                renderCaughtUp(container);
            }

        } catch (err) {
            container.querySelectorAll('.skeleton-post').forEach(function (sk) { sk.remove(); });
            console.error('Feed load error:', err);

            if (err.code === -32000 || err.code === -32002) {
                renderNetworkError(container);
            } else {
                renderEmptyFeed(container);
            }
        }
    }

    // Clean up when leaving feed
    Ephemera.store.subscribe(function (state) {
        if (state.route !== '/feed') {
            if (observer) { observer.disconnect(); observer = null; }
        }
    });

    Ephemera.registerRoute('/feed', renderFeed);
})();
