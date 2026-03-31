/**
 * Ephemera -- Profile View
 *
 * Full profile page with:
 * - Large avatar (tappable for photo upload)
 * - Display name (tappable inline edit)
 * - @handle with registration flow
 * - Bio (tappable inline edit)
 * - Node ID (copyable)
 * - "Edit Profile" button
 * - Recent posts
 * - Gear icon -> settings sub-page
 *
 * RPC calls:
 *   - identity.get_active {}
 *   - profiles.update { display_name, bio }
 *   - identity.register_handle { handle }
 *   - identity.lookup_handle { handle }
 *   - feed.own { limit }
 *   - meta.status {}
 *   - meta.set_transport_tier { tier }
 *   - identity.backup_mnemonic { passphrase }
 *   - identity.lock {}
 *   - network.peers {}
 *   - network.connect { addr }
 *   - network.disconnect { peer_id }
 */
(function () {
    'use strict';

    var showingSettings = false;

    async function renderProfile(container) {
        container.innerHTML = '';
        showingSettings = false;

        var state = Ephemera.store.get();
        var identity = state.identity || {};

        // Show banner if handle registration is in progress
        if (state.handleRegistering && state.registeringHandle) {
            var regBanner = Ephemera.el('div', 'handle-registering-banner');
            regBanner.style.cssText = 'background:var(--bg-surface-2);border:1px solid var(--accent);border-radius:12px;padding:12px 16px;margin-bottom:16px;display:flex;align-items:center;gap:10px;';
            var spin = Ephemera.el('div', 'spinner');
            spin.style.cssText = 'width:16px;height:16px;flex-shrink:0;';
            regBanner.appendChild(spin);
            regBanner.appendChild(Ephemera.el('span', '', 'Registering @' + state.registeringHandle + '... You can keep using the app.'));
            container.appendChild(regBanner);
        }

        // Profile header
        var profileHeader = Ephemera.el('div', 'profile-header');

        // Settings gear button (top right)
        var gearBtn = Ephemera.el('button', 'btn btn-ghost btn-sm');
        gearBtn.style.cssText = 'position:absolute;top:var(--sp-2);right:0;';
        gearBtn.innerHTML = '<svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>';
        gearBtn.setAttribute('aria-label', 'Settings');
        gearBtn.addEventListener('click', function () {
            renderSettingsPage(container);
        });
        profileHeader.appendChild(gearBtn);

        // Avatar (tappable)
        var avatarWrap = Ephemera.el('div', 'profile-avatar-wrap');
        avatarWrap.appendChild(Ephemera.avatar(identity.display_name || '?', 'avatar-2xl', identity.avatar_url || null));
        var editHint = Ephemera.el('div', 'profile-avatar-edit-hint');
        editHint.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M23 19a2 2 0 0 1-2 2H3a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4l2-3h6l2 3h4a2 2 0 0 1 2 2z"/><circle cx="12" cy="13" r="4"/></svg>';
        avatarWrap.appendChild(editHint);
        avatarWrap.setAttribute('aria-label', 'Change profile photo');
        avatarWrap.addEventListener('click', function () {
            var input = document.createElement('input');
            input.type = 'file';
            input.accept = 'image/jpeg,image/png,image/webp';
            input.style.cssText = 'position:fixed;top:-9999px;left:-9999px;opacity:0;';
            document.body.appendChild(input); // Must be in DOM for some WebViews
            input.onchange = function () {
                document.body.removeChild(input); // Clean up
                if (!input.files || !input.files[0]) return;
                var file = input.files[0];
                if (file.size > 5 * 1024 * 1024) {
                    Ephemera.showToast('Image must be under 5MB', 'error');
                    return;
                }
                // Direct center-crop upload (skips broken crop modal)
                var img = new Image();
                var reader = new FileReader();
                reader.onload = function (e) {
                    img.onload = async function () {
                        var canvas = document.createElement('canvas');
                        canvas.width = 512;
                        canvas.height = 512;
                        var ctx = canvas.getContext('2d');
                        var size = Math.min(img.width, img.height);
                        var sx = (img.width - size) / 2;
                        var sy = (img.height - size) / 2;
                        ctx.drawImage(img, sx, sy, size, size, 0, 0, 512, 512);
                        canvas.toBlob(async function (blob) {
                            if (!blob) {
                                Ephemera.showToast('Failed to process image', 'error');
                                return;
                            }
                            try {
                                var buf = await blob.arrayBuffer();
                                var hex = Array.from(new Uint8Array(buf)).map(function (b) {
                                    return b.toString(16).padStart(2, '0');
                                }).join('');
                                var result = await Ephemera.rpc('profiles.update_avatar', { data_hex: hex, filename: 'avatar.jpg' });
                                identity.avatar_url = result.avatar_url;
                                Ephemera.store.set({ identity: identity });
                                Ephemera.showToast('Avatar updated!', 'success');
                                renderProfile(container);
                            } catch (err) {
                                Ephemera.showToast('Upload failed: ' + err.message, 'error');
                            }
                        }, 'image/jpeg', 0.85);
                    };
                    img.src = e.target.result;
                };
                reader.onerror = function () {
                    Ephemera.showToast('Could not read image file', 'error');
                };
                reader.readAsDataURL(file);
            };
            input.click();
        });
        profileHeader.appendChild(avatarWrap);

        // Display name (tappable to edit)
        var nameEl = Ephemera.el('div', 'profile-display-name', identity.display_name || 'Set your name');
        nameEl.setAttribute('aria-label', 'Edit display name');
        nameEl.addEventListener('click', function () {
            startInlineEdit(nameEl, identity.display_name || '', 30, async function (newVal) {
                if (!newVal.trim()) return;
                try {
                    await Ephemera.rpc('profiles.update', { display_name: newVal.trim() });
                    identity.display_name = newVal.trim();
                    Ephemera.store.set({ identity: identity });
                    Ephemera.showToast('Name updated!', 'success');
                } catch (err) {
                    Ephemera.showToast('Failed: ' + err.message, 'error');
                }
            });
        });
        profileHeader.appendChild(nameEl);

        // @handle
        var handleDisplay = Ephemera.getDisplayHandle(identity);
        if (handleDisplay) {
            var handleEl = Ephemera.el('div', 'profile-handle', handleDisplay);
            handleEl.addEventListener('click', function () {
                showHandleRegistration(container, identity);
            });
            profileHeader.appendChild(handleEl);
        }

        // Bio (tappable to edit)
        var bioText = identity.bio || '';
        var bioEl = Ephemera.el('div', 'profile-bio' + (bioText ? '' : ' placeholder'),
            bioText || 'Tap to add a bio...');
        bioEl.setAttribute('aria-label', 'Edit bio');
        bioEl.addEventListener('click', function () {
            showBioEditModal(identity, container);
        });
        profileHeader.appendChild(bioEl);

        // Node ID (copyable)
        var pk = identity.pseudonym_id || identity.public_key || identity.pubkey || '';
        var shortPk = pk.length > 20 ? pk.slice(0, 10) + '...' + pk.slice(-8) : pk || 'Unknown';
        var nodeIdEl = Ephemera.el('div', 'profile-node-id');
        nodeIdEl.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>';
        nodeIdEl.appendChild(document.createTextNode(shortPk));
        nodeIdEl.setAttribute('aria-label', 'Copy node ID');
        nodeIdEl.addEventListener('click', function () {
            if (navigator.clipboard && navigator.clipboard.writeText && pk) {
                navigator.clipboard.writeText(pk).then(function () {
                    Ephemera.showToast('Node ID copied!', 'success');
                }).catch(function () {
                    Ephemera.showToast('Copy failed', 'error');
                });
            }
        });
        profileHeader.appendChild(nodeIdEl);

        // Edit Profile button + Handle registration CTA
        var editBtn = Ephemera.el('button', 'btn btn-secondary btn-sm', 'Edit Profile');
        editBtn.addEventListener('click', function () {
            showEditProfileModal(identity, container);
        });
        profileHeader.appendChild(editBtn);

        container.appendChild(profileHeader);

        // Handle registration CTA (if no handle)
        if (!identity.handle) {
            var handleCta = Ephemera.el('div', 'handle-cta');
            handleCta.appendChild(Ephemera.el('h3', '', 'Choose your @handle'));
            handleCta.appendChild(Ephemera.el('p', '',
                'A unique handle makes it easy for others to find and mention you.'));

            var handleRow = Ephemera.el('div', 'handle-input-row');
            var handleInputWrap = Ephemera.el('div', 'handle-input-wrap');
            handleInputWrap.appendChild(Ephemera.el('span', 'at-prefix', '@'));

            var handleInput = document.createElement('input');
            handleInput.type = 'text';
            handleInput.className = 'input-field';
            handleInput.placeholder = 'yourname';
            handleInput.maxLength = 20;
            handleInput.setAttribute('aria-label', 'Choose a handle');
            handleInputWrap.appendChild(handleInput);
            handleRow.appendChild(handleInputWrap);

            var registerBtn = Ephemera.el('button', 'btn btn-primary btn-sm', 'Register');
            handleRow.appendChild(registerBtn);
            handleCta.appendChild(handleRow);

            var charCount = Ephemera.el('div', 'handle-char-count', '0/20');
            handleCta.appendChild(charCount);

            var validationMsg = Ephemera.el('div', 'handle-validation');
            handleCta.appendChild(validationMsg);

            handleInput.addEventListener('input', function () {
                var val = handleInput.value.replace(/[^a-zA-Z0-9_]/g, '').toLowerCase();
                handleInput.value = val;
                charCount.textContent = val.length + '/20';

                if (val.length === 0) {
                    validationMsg.textContent = '';
                    validationMsg.className = 'handle-validation';
                } else if (val.length < 3) {
                    validationMsg.textContent = 'Must be at least 3 characters';
                    validationMsg.className = 'handle-validation error';
                } else if (/^[a-z][a-z0-9_]{2,19}$/.test(val)) {
                    validationMsg.textContent = 'Looks good!';
                    validationMsg.className = 'handle-validation ok';
                } else {
                    validationMsg.textContent = 'Must start with a letter, only letters/numbers/underscores';
                    validationMsg.className = 'handle-validation error';
                }
            });

            // PoW status area (below the register button)
            var powStatusEl = Ephemera.el('div', 'handle-pow-status');
            powStatusEl.style.cssText = 'margin-top:8px;font-size:0.85em;color:var(--text-secondary);';
            handleCta.appendChild(powStatusEl);

            registerBtn.addEventListener('click', async function () {
                var handle = handleInput.value.trim().toLowerCase();
                if (handle.length < 3 || !/^[a-z][a-z0-9_]{2,19}$/.test(handle)) {
                    Ephemera.showToast('Invalid handle format', 'error');
                    return;
                }

                registerBtn.disabled = true;
                handleInput.disabled = true;

                // Compute difficulty info for user feedback
                var difficultyInfo = '';
                var estimatedSecs = 5;
                if (handle.length <= 5) {
                    difficultyInfo = 'Short handles require more computation to prevent squatting. This may take up to 5 minutes.';
                    estimatedSecs = handle.length <= 3 ? 300 : handle.length <= 4 ? 120 : 60;
                } else if (handle.length <= 10) {
                    difficultyInfo = 'This will take about 30 seconds.';
                    estimatedSecs = 30;
                } else {
                    difficultyInfo = 'This will take about 5 seconds.';
                    estimatedSecs = 5;
                }

                powStatusEl.textContent = difficultyInfo;
                registerBtn.textContent = 'Computing proof-of-work...';

                // Store registration state so it persists across navigation
                Ephemera.store.set({ registeringHandle: handle, handleRegistering: true });

                // Show spinner
                var spinnerEl = Ephemera.el('div', 'spinner');
                spinnerEl.style.cssText = 'display:inline-block;width:16px;height:16px;margin-left:8px;vertical-align:middle;';
                registerBtn.appendChild(spinnerEl);

                // Fire and forget — don't await here, let it run in background
                Ephemera.rpc('identity.register_handle', { name: handle }).then(function () {
                    // Success! Update state globally
                    var currentIdentity = Ephemera.store.get().identity || {};
                    currentIdentity.handle = handle;
                    Ephemera.store.set({
                        identity: currentIdentity,
                        registeringHandle: null,
                        handleRegistering: false,
                    });

                    var renewDate = new Date(Date.now() + 90 * 24 * 60 * 60 * 1000);
                    var renewStr = renewDate.toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
                    Ephemera.showToast('Handle @' + handle + ' registered! Renew before ' + renewStr + '.', 'success');

                    // Re-render profile if we're still on it
                    if (Ephemera.getCurrentRoute && Ephemera.getCurrentRoute() === '/profile') {
                        renderProfile(container);
                    }
                }).catch(function (err) {
                    Ephemera.store.set({ registeringHandle: null, handleRegistering: false });
                    Ephemera.showToast('Registration failed: ' + err.message, 'error');
                });

                // Show a persistent message that stays even if user navigates away
                Ephemera.showToast('Registering @' + handle + '... This runs in the background. You can continue using the app.', 'info');
            });

            container.appendChild(handleCta);
        }

        // Handle conflict detection: poll for handle status changes.
        // If we have a handle, periodically check it's still ours.
        if (identity.handle) {
            var conflictPollId = setInterval(async function () {
                // Stop polling if we navigated away from profile
                if (!document.body.contains(container)) {
                    clearInterval(conflictPollId);
                    return;
                }
                try {
                    var status = await Ephemera.rpc('identity.check_handle_status', {});
                    if (status && status.active === false) {
                        clearInterval(conflictPollId);
                        var lostHandle = identity.handle;
                        identity.handle = null;
                        Ephemera.store.set({ identity: identity });
                        Ephemera.showToast(
                            'Your handle @' + lostHandle + ' was claimed by someone who registered it first. Tap to choose a new handle.',
                            'error'
                        );
                        renderProfile(container);
                    }
                } catch (_err) {
                    // Silently ignore polling errors (node may be offline).
                }
            }, 15000); // Poll every 15 seconds
        }

        // Mentions section
        renderMentionsInProfile(container);

        // Recent posts section
        var postsSection = Ephemera.el('div', 'profile-section');
        var postsSectionHeader = Ephemera.el('div', 'profile-section-header');
        postsSectionHeader.appendChild(Ephemera.el('h2', '', 'Your Posts'));
        postsSection.appendChild(postsSectionHeader);

        var postsLoading = Ephemera.el('div', 'loading-state');
        postsLoading.appendChild(Ephemera.el('div', 'spinner'));
        postsSection.appendChild(postsLoading);
        container.appendChild(postsSection);

        // Load user's posts
        try {
            var result = await Ephemera.rpc('feed.own', { limit: 10 });
            postsSection.removeChild(postsLoading);

            var posts = result.posts || result || [];
            if (!Array.isArray(posts) || posts.length === 0) {
                var emptyPosts = Ephemera.el('div', 'empty-state');
                emptyPosts.style.padding = 'var(--sp-8) var(--sp-4)';
                emptyPosts.appendChild(Ephemera.el('p', '', 'You haven\'t posted anything yet. Your posts will appear here.'));

                var createBtn = Ephemera.el('button', 'btn btn-primary btn-sm', 'Create a Post');
                createBtn.addEventListener('click', function () {
                    Ephemera.store.set({ replyTo: null });
                    Ephemera.navigate('/compose');
                });
                emptyPosts.appendChild(createBtn);
                postsSection.appendChild(emptyPosts);
            } else {
                posts.forEach(function (raw) {
                    var post = normalizeOwnPost(raw);
                    postsSection.appendChild(renderMiniPostCard(post));
                });
            }
        } catch (_err) {
            postsSection.removeChild(postsLoading);
            var emptyPosts = Ephemera.el('div', 'empty-state');
            emptyPosts.style.padding = 'var(--sp-8) var(--sp-4)';
            emptyPosts.appendChild(Ephemera.el('p', '', 'Your posts will appear here.'));
            postsSection.appendChild(emptyPosts);
        }
    }

    async function renderMentionsInProfile(container) {
        try {
            var result = await Ephemera.rpc('mentions.list', { limit: 10 });
            var mentions = result.mentions || result.items || [];
            if (!Array.isArray(mentions) || mentions.length === 0) return;

            var section = Ephemera.el('div', 'profile-section');
            var header = Ephemera.el('div', 'profile-section-header');
            var titleRow = Ephemera.el('div', 'section-title-row');
            titleRow.style.marginTop = '0';
            var title = Ephemera.el('h2', '', 'Mentions');
            var badge = Ephemera.el('span', 'mentions-badge', String(mentions.length));
            titleRow.appendChild(title);
            titleRow.appendChild(badge);
            header.appendChild(titleRow);
            section.appendChild(header);

            mentions.forEach(function (m) {
                var row = Ephemera.el('div', 'mention-row');
                var author = m.author_handle ? '@' + m.author_handle : 'Someone';
                row.appendChild(Ephemera.el('span', 'mention-author', author));
                row.appendChild(Ephemera.el('span', 'mention-text', ' mentioned you'));
                if (m.body_preview) {
                    var preview = Ephemera.el('div', 'mention-preview');
                    preview.textContent = m.body_preview.length > 100
                        ? m.body_preview.slice(0, 100) + '...'
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
            // Silently skip if mentions not supported
        }
    }

    function normalizeOwnPost(raw) {
        var now = Date.now();
        var createdAtMs = raw.created_at;
        if (createdAtMs && createdAtMs < 1e12) createdAtMs = createdAtMs * 1000;
        var expiresAtMs = raw.expires_at;
        if (expiresAtMs && expiresAtMs < 1e12) expiresAtMs = expiresAtMs * 1000;
        var remainingSecs = expiresAtMs ? (expiresAtMs - now) / 1000 : 2592000;

        return {
            content_hash: raw.content_hash || '',
            body: raw.body || raw.body_preview || '',
            created_at: createdAtMs || now,
            remaining_seconds: remainingSecs,
            ttl_seconds: raw.ttl_seconds || 2592000,
            audience: raw.audience || 'everyone',
            reply_count: raw.reply_count || 0,
        };
    }

    function renderMiniPostCard(post) {
        var card = Ephemera.el('div', 'post-card');
        card.style.cursor = 'default';

        var pct = Ephemera.ttlPercent(post.remaining_seconds, post.ttl_seconds);

        if (post.body) {
            var bodyEl = Ephemera.el('div', 'post-body', post.body);
            bodyEl.style.cssText = 'font-size:var(--fs-sm);display:-webkit-box;-webkit-line-clamp:3;-webkit-box-orient:vertical;overflow:hidden;';
            card.appendChild(bodyEl);
        }

        var metaRow = Ephemera.el('div', 'post-meta-row');
        metaRow.style.marginTop = 'var(--sp-2)';

        var badge = Ephemera.el('span', 'audience-badge');
        badge.innerHTML = Ephemera.audienceIcon(post.audience) + ' ' + Ephemera.audienceLabel(post.audience);
        metaRow.appendChild(badge);

        metaRow.appendChild(Ephemera.el('span', 'post-expiry', Ephemera.formatTTL(post.remaining_seconds)));

        if (post.reply_count > 0) {
            var replies = Ephemera.el('span', 'post-expiry');
            replies.innerHTML = '<svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2"><path d="M21 11.5a8.38 8.38 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.38 8.38 0 0 1-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.38 8.38 0 0 1 3.8-.9h.5a8.48 8.48 0 0 1 8 8v.5z"/></svg> ' + post.reply_count;
            metaRow.appendChild(replies);
        }

        metaRow.appendChild(Ephemera.el('span', 'post-timestamp', Ephemera.timeAgo(post.created_at)));

        card.appendChild(metaRow);

        // TTL bar
        var ttlBar = Ephemera.el('div', 'ttl-bar');
        var ttlFill = Ephemera.el('div', 'ttl-bar-fill');
        ttlFill.style.width = (pct * 100).toFixed(1) + '%';
        if (pct > 0.5) ttlFill.classList.add('fresh');
        else if (pct > 0.1) ttlFill.classList.add('aging');
        else ttlFill.classList.add('critical');
        ttlBar.appendChild(ttlFill);
        card.appendChild(ttlBar);

        return card;
    }

    // ---- Inline editing ----

    function startInlineEdit(element, currentValue, maxLength, onSave) {
        var originalText = element.textContent;
        var input = document.createElement('input');
        input.type = 'text';
        input.className = 'inline-edit';
        input.value = currentValue;
        input.maxLength = maxLength;

        element.textContent = '';
        element.appendChild(input);
        input.focus();
        input.select();

        function finish(save) {
            var val = input.value.trim();
            element.textContent = val || originalText;
            if (save && val && val !== currentValue) {
                onSave(val);
            }
        }

        input.addEventListener('blur', function () { finish(true); });
        input.addEventListener('keydown', function (e) {
            if (e.key === 'Enter') { e.preventDefault(); input.blur(); }
            if (e.key === 'Escape') { e.preventDefault(); finish(false); }
        });
    }

    // ---- Modals ----

    function showBioEditModal(identity, parentContainer) {
        var overlay = Ephemera.el('div', 'modal-overlay');
        overlay.setAttribute('role', 'dialog');
        overlay.setAttribute('aria-modal', 'true');

        var modal = Ephemera.el('div', 'modal-content');
        modal.appendChild(Ephemera.el('h2', '', 'Edit Bio'));

        var group = Ephemera.el('div', 'input-group');
        var textarea = document.createElement('textarea');
        textarea.className = 'input-field';
        textarea.value = identity.bio || '';
        textarea.placeholder = 'Tell people about yourself...';
        textarea.maxLength = 160;
        textarea.rows = 4;
        group.appendChild(textarea);

        var charCount = Ephemera.el('div', '');
        charCount.style.cssText = 'font-size:var(--fs-xs);color:var(--text-tertiary);text-align:right;margin-top:4px;';
        charCount.textContent = (identity.bio || '').length + '/160';
        textarea.addEventListener('input', function () {
            charCount.textContent = textarea.value.length + '/160';
        });
        group.appendChild(charCount);
        modal.appendChild(group);

        var actionsRow = Ephemera.el('div', 'modal-actions');
        var cancelBtn = Ephemera.el('button', 'btn btn-ghost', 'Cancel');
        cancelBtn.addEventListener('click', function () { overlay.remove(); });
        actionsRow.appendChild(cancelBtn);

        var saveBtn = Ephemera.el('button', 'btn btn-primary', 'Save');
        saveBtn.addEventListener('click', async function () {
            saveBtn.disabled = true;
            saveBtn.textContent = 'Saving...';
            try {
                await Ephemera.rpc('profiles.update', { bio: textarea.value.trim() });
                identity.bio = textarea.value.trim();
                Ephemera.store.set({ identity: identity });
                Ephemera.showToast('Bio updated!', 'success');
                overlay.remove();
                renderProfile(parentContainer);
            } catch (err) {
                Ephemera.showToast('Failed: ' + err.message, 'error');
                saveBtn.disabled = false;
                saveBtn.textContent = 'Save';
            }
        });
        actionsRow.appendChild(saveBtn);
        modal.appendChild(actionsRow);
        overlay.appendChild(modal);

        overlay.addEventListener('click', function (e) { if (e.target === overlay) overlay.remove(); });
        function onEsc(e) { if (e.key === 'Escape') { overlay.remove(); document.removeEventListener('keydown', onEsc); } }
        document.addEventListener('keydown', onEsc);

        document.body.appendChild(overlay);
        textarea.focus();
    }

    function showEditProfileModal(identity, parentContainer) {
        var overlay = Ephemera.el('div', 'modal-overlay');
        overlay.setAttribute('role', 'dialog');
        overlay.setAttribute('aria-modal', 'true');

        var modal = Ephemera.el('div', 'modal-content');
        modal.appendChild(Ephemera.el('h2', '', 'Edit Profile'));

        // Display name
        var nameGroup = Ephemera.el('div', 'input-group');
        nameGroup.appendChild(Ephemera.el('label', '', 'Display Name'));
        var nameInput = document.createElement('input');
        nameInput.type = 'text';
        nameInput.className = 'input-field';
        nameInput.maxLength = 30;
        nameInput.value = identity.display_name || '';
        nameInput.placeholder = '1-30 characters';
        nameGroup.appendChild(nameInput);
        modal.appendChild(nameGroup);

        // Bio
        var bioGroup = Ephemera.el('div', 'input-group');
        bioGroup.appendChild(Ephemera.el('label', '', 'Bio'));
        var bioInput = document.createElement('textarea');
        bioInput.className = 'input-field';
        bioInput.maxLength = 200;
        bioInput.value = identity.bio || '';
        bioInput.placeholder = 'Tell people about yourself...';
        bioInput.rows = 3;
        bioGroup.appendChild(bioInput);
        modal.appendChild(bioGroup);

        var actionsRow = Ephemera.el('div', 'modal-actions');
        var cancelBtn = Ephemera.el('button', 'btn btn-ghost', 'Cancel');
        cancelBtn.addEventListener('click', function () { overlay.remove(); });
        actionsRow.appendChild(cancelBtn);

        var saveBtn = Ephemera.el('button', 'btn btn-primary', 'Save');
        saveBtn.addEventListener('click', async function () {
            var newName = nameInput.value.trim();
            if (!newName) {
                Ephemera.showToast('Name cannot be empty', 'error');
                return;
            }
            saveBtn.disabled = true;
            saveBtn.textContent = 'Saving...';
            try {
                await Ephemera.rpc('profiles.update', {
                    display_name: newName,
                    bio: bioInput.value.trim(),
                });
                identity.display_name = newName;
                identity.bio = bioInput.value.trim();
                Ephemera.store.set({ identity: identity });
                Ephemera.showToast('Profile updated!', 'success');
                overlay.remove();
                renderProfile(parentContainer);
            } catch (err) {
                Ephemera.showToast('Failed: ' + err.message, 'error');
                saveBtn.disabled = false;
                saveBtn.textContent = 'Save';
            }
        });
        actionsRow.appendChild(saveBtn);
        modal.appendChild(actionsRow);
        overlay.appendChild(modal);

        overlay.addEventListener('click', function (e) { if (e.target === overlay) overlay.remove(); });
        function onEsc(e) { if (e.key === 'Escape') { overlay.remove(); document.removeEventListener('keydown', onEsc); } }
        document.addEventListener('keydown', onEsc);

        document.body.appendChild(overlay);
        nameInput.focus();
        nameInput.select();
    }

    function showHandleRegistration(parentContainer, identity) {
        // Scroll to handle CTA if it exists
        var cta = parentContainer.querySelector('.handle-cta');
        if (cta) {
            cta.scrollIntoView({ behavior: 'smooth', block: 'center' });
            var input = cta.querySelector('input');
            if (input) input.focus();
        }
    }

    // ---- Avatar Crop Modal ----

    function showAvatarCropModal(file, identity, parentContainer) {
        var overlay = Ephemera.el('div', 'modal-overlay');
        overlay.setAttribute('role', 'dialog');
        overlay.setAttribute('aria-modal', 'true');
        overlay.setAttribute('aria-label', 'Crop avatar');

        // Close on Android back button
        function onBackButton(e) {
            if (e.key === 'Escape' || e.key === 'Back') { closeModal(); }
        }
        function closeModal() {
            overlay.remove();
            document.removeEventListener('keydown', onBackButton);
            window.removeEventListener('popstate', closeModal);
        }
        document.addEventListener('keydown', onBackButton);
        // Push a history state so Android back button triggers popstate
        history.pushState({ modal: 'crop' }, '');
        window.addEventListener('popstate', closeModal);

        // Tap overlay background to close
        overlay.addEventListener('click', function (e) {
            if (e.target === overlay) closeModal();
        });

        var modal = Ephemera.el('div', 'modal-content');

        var cropContent = Ephemera.el('div', 'avatar-crop-modal');

        // Header with Cancel and Save buttons visible at top
        var header = Ephemera.el('div', '');
        header.style.cssText = 'display:flex;justify-content:space-between;align-items:center;width:100%;margin-bottom:12px;';
        var cancelBtn = Ephemera.el('button', 'btn btn-ghost', 'Cancel');
        cancelBtn.addEventListener('click', closeModal);
        header.appendChild(cancelBtn);
        header.appendChild(Ephemera.el('span', '', 'Adjust Photo'));
        var saveBtn = Ephemera.el('button', 'btn btn-primary btn-sm', 'Save');
        header.appendChild(saveBtn);
        cropContent.appendChild(header);

        // Simple preview: show image in a circle with object-fit cover
        // No drag/reposition (unreliable on mobile) — just center-crop
        var preview = Ephemera.el('div', '');
        preview.style.cssText = 'width:220px;height:220px;border-radius:50%;overflow:hidden;border:3px solid var(--accent);margin:0 auto;background:var(--bg-surface-2);';
        var img = document.createElement('img');
        img.alt = 'Avatar preview';
        img.style.cssText = 'width:100%;height:100%;object-fit:cover;display:block;';

        // Read file as data URL
        var reader = new FileReader();
        reader.onload = function (e) {
            img.src = e.target.result;
        };
        reader.onerror = function () {
            Ephemera.showToast('Could not load image', 'error');
            closeModal();
        };
        reader.readAsDataURL(file);

        preview.appendChild(img);
        cropContent.appendChild(preview);

        cropContent.appendChild(Ephemera.el('div', 'avatar-crop-hint', 'Your photo will be center-cropped to a circle.'));

        modal.appendChild(cropContent);

        // Save button handler
        saveBtn.addEventListener('click', async function () {
            saveBtn.disabled = true;
            saveBtn.textContent = 'Uploading...';

            try {
                // Wait for image to load
                if (!img.naturalWidth) {
                    await new Promise(function (resolve) {
                        img.onload = resolve;
                        setTimeout(resolve, 3000); // timeout fallback
                    });
                }

                // Draw center-cropped square to canvas
                var canvas = document.createElement('canvas');
                var size = 512;
                canvas.width = size;
                canvas.height = size;
                var ctx = canvas.getContext('2d');

                var imgW = img.naturalWidth || 512;
                var imgH = img.naturalHeight || 512;
                var cropSize = Math.min(imgW, imgH);
                var sx = (imgW - cropSize) / 2;
                var sy = (imgH - cropSize) / 2;

                ctx.drawImage(img, sx, sy, cropSize, cropSize, 0, 0, size, size);

                var blob = await new Promise(function (resolve) {
                    canvas.toBlob(resolve, 'image/jpeg', 0.9);
                });

                var buf = await blob.arrayBuffer();
                var hex = Array.from(new Uint8Array(buf)).map(function (b) {
                    return b.toString(16).padStart(2, '0');
                }).join('');

                var result = await Ephemera.rpc('profiles.update_avatar', { data_hex: hex, filename: 'avatar.jpg' });
                identity.avatar_url = result.avatar_url;
                Ephemera.store.set({ identity: identity });
                Ephemera.showToast('Avatar updated!', 'success');
                closeModal();
                renderProfile(parentContainer);
            } catch (err) {
                Ephemera.showToast('Upload failed: ' + err.message, 'error');
                saveBtn.disabled = false;
                saveBtn.textContent = 'Save';
            }
        });

        overlay.appendChild(modal);
        document.body.appendChild(overlay);
    }

    // ================================================================
    // Settings Sub-Page
    // ================================================================

    async function renderSettingsPage(container) {
        container.innerHTML = '';
        showingSettings = true;

        var state = Ephemera.store.get();
        var identity = state.identity || {};

        // Header with back button
        var header = Ephemera.el('div', 'discover-header');
        var backBtn = Ephemera.el('button', 'btn btn-ghost btn-sm');
        backBtn.innerHTML = '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2"><polyline points="15 18 9 12 15 6"/></svg> Back';
        backBtn.addEventListener('click', function () {
            renderProfile(container);
        });
        header.appendChild(backBtn);
        header.appendChild(Ephemera.el('h1', '', 'Settings'));
        container.appendChild(header);

        // ---- Privacy Section ----
        var privacySection = Ephemera.el('div', 'settings-section');
        privacySection.style.borderTop = 'none';
        privacySection.style.marginTop = '0';
        privacySection.style.paddingTop = '0';
        privacySection.appendChild(Ephemera.el('h2', '', 'Privacy'));

        var tierDesc = Ephemera.el('p', '', 'How your network traffic is routed:');
        tierDesc.style.cssText = 'font-size:var(--fs-sm);color:var(--text-secondary);margin-bottom:12px;';
        privacySection.appendChild(tierDesc);

        var tierSelector = Ephemera.el('div', 'privacy-tier-selector');
        tierSelector.setAttribute('role', 'radiogroup');

        var tiers = [
            { id: 'fast',    name: 'Fast',    desc: 'Single relay' },
            { id: 'private', name: 'Private', desc: 'Tor 3-hop' },
            { id: 'stealth', name: 'Stealth', desc: 'Mixnet' },
        ];

        var currentTier = identity.privacy_tier || 'private';

        tiers.forEach(function (tier) {
            var btn = Ephemera.el('button', 'privacy-tier-btn');
            btn.setAttribute('role', 'radio');
            btn.setAttribute('aria-checked', tier.id === currentTier ? 'true' : 'false');
            if (tier.id === currentTier) btn.classList.add('active');

            btn.appendChild(Ephemera.el('span', 'tier-name', tier.name));
            btn.appendChild(Ephemera.el('span', 'tier-desc', tier.desc));

            btn.addEventListener('click', async function () {
                tierSelector.querySelectorAll('.privacy-tier-btn').forEach(function (b) {
                    b.classList.remove('active');
                    b.setAttribute('aria-checked', 'false');
                });
                btn.classList.add('active');
                btn.setAttribute('aria-checked', 'true');
                try {
                    await Ephemera.rpc('meta.set_transport_tier', { tier: tier.id });
                    Ephemera.showToast('Privacy: ' + tier.name, 'success');
                } catch (err) {
                    Ephemera.showToast('Failed: ' + err.message, 'error');
                }
            });

            tierSelector.appendChild(btn);
        });

        privacySection.appendChild(tierSelector);
        container.appendChild(privacySection);

        // ---- Storage Section ----
        var storageSection = Ephemera.el('div', 'settings-section');
        storageSection.appendChild(Ephemera.el('h2', '', 'Storage'));

        var storageGroup = Ephemera.el('div', 'settings-group');
        var storageRow = Ephemera.el('div', 'settings-row');
        storageRow.style.cssText = 'flex-direction:column;align-items:stretch;';

        var storageUsed = '0 MB';
        var storageTotal = '500 MB';
        var usagePct = 0;

        try {
            var status = await Ephemera.rpc('meta.status', {});
            if (status) {
                var usedBytes = status.storage_used_bytes || 0;
                var maxBytes = status.storage_cap_bytes || status.storage_max_bytes || (500 * 1024 * 1024);
                storageUsed = formatBytes(usedBytes);
                storageTotal = formatBytes(maxBytes);
                usagePct = maxBytes > 0 ? (usedBytes / maxBytes) * 100 : 0;
            }
        } catch (_e) { /* use defaults */ }

        var storageLabelRow = Ephemera.el('div', '');
        storageLabelRow.style.cssText = 'display:flex;justify-content:space-between;margin-bottom:4px;';
        storageLabelRow.appendChild(Ephemera.el('span', 'settings-row-label', 'Local Storage'));
        storageLabelRow.appendChild(Ephemera.el('span', 'settings-row-value',
            storageUsed + ' / ' + storageTotal));
        storageRow.appendChild(storageLabelRow);

        var bar = Ephemera.el('div', 'storage-bar');
        var barFill = Ephemera.el('div', 'storage-bar-fill');
        barFill.style.width = Math.min(usagePct, 100) + '%';
        bar.appendChild(barFill);
        storageRow.appendChild(bar);

        storageGroup.appendChild(storageRow);
        storageSection.appendChild(storageGroup);
        container.appendChild(storageSection);

        // ---- Security Section ----
        var securitySection = Ephemera.el('div', 'settings-section');
        securitySection.appendChild(Ephemera.el('h2', '', 'Security'));

        var secGroup = Ephemera.el('div', 'settings-group');

        var recoveryRow = Ephemera.el('div', 'settings-row');
        recoveryRow.appendChild(Ephemera.el('span', 'settings-row-label', 'Recovery Phrase'));
        var exportBtn = Ephemera.el('button', 'btn btn-ghost btn-sm', 'Export');
        exportBtn.addEventListener('click', function () { showRecoveryPhraseModal(); });
        recoveryRow.appendChild(exportBtn);
        secGroup.appendChild(recoveryRow);

        var importRow = Ephemera.el('div', 'settings-row');
        importRow.appendChild(Ephemera.el('span', 'settings-row-label', 'Import Identity'));
        var importBtn = Ephemera.el('button', 'btn btn-ghost btn-sm', 'Scan QR');
        importBtn.addEventListener('click', function () { showImportQrModal(container); });
        importRow.appendChild(importBtn);
        secGroup.appendChild(importRow);

        securitySection.appendChild(secGroup);
        container.appendChild(securitySection);

        // ---- Network Section ----
        var networkSection = Ephemera.el('div', 'settings-section');
        networkSection.appendChild(Ephemera.el('h2', '', 'Network'));

        // Network status diagnostic panel
        var statusGroup = Ephemera.el('div', 'settings-group');
        var statusWrap = Ephemera.el('div', '');
        statusWrap.style.padding = 'var(--sp-3) var(--sp-4)';
        var statusLoading = Ephemera.el('p', '');
        statusLoading.style.cssText = 'font-size:var(--fs-sm);color:var(--text-secondary);';
        statusLoading.textContent = 'Loading network status...';
        statusWrap.appendChild(statusLoading);
        statusGroup.appendChild(statusWrap);
        networkSection.appendChild(statusGroup);
        renderNetworkStatus(statusWrap);

        var netGroup = Ephemera.el('div', 'settings-group');
        netGroup.style.marginTop = '12px';
        var peerListWrap = Ephemera.el('div', '');
        peerListWrap.style.padding = 'var(--sp-3) var(--sp-4)';
        var peerLoading = Ephemera.el('p', '');
        peerLoading.style.cssText = 'font-size:var(--fs-sm);color:var(--text-secondary);';
        peerLoading.textContent = 'Loading peers...';
        peerListWrap.appendChild(peerLoading);
        netGroup.appendChild(peerListWrap);
        networkSection.appendChild(netGroup);

        var addPeerRow = Ephemera.el('div', '');
        addPeerRow.style.cssText = 'display:flex;align-items:center;gap:8px;margin-top:12px;';

        var peerInput = document.createElement('input');
        peerInput.type = 'text';
        peerInput.className = 'input-field';
        peerInput.placeholder = 'IP:port (e.g. 192.168.1.10:9100)';
        peerInput.style.flex = '1';
        addPeerRow.appendChild(peerInput);

        var connectPeerBtn = Ephemera.el('button', 'btn btn-primary btn-sm', 'Connect');
        connectPeerBtn.addEventListener('click', async function () {
            var addr = peerInput.value.trim();
            if (!addr) { Ephemera.showToast('Enter a peer address', 'error'); return; }
            connectPeerBtn.disabled = true;
            connectPeerBtn.textContent = 'Connecting...';
            try {
                await Ephemera.rpc('network.connect', { addr: addr });
                Ephemera.showToast('Connected to ' + addr, 'success');
                peerInput.value = '';
                refreshPeerList(peerListWrap);
            } catch (err) {
                Ephemera.showToast('Failed: ' + err.message, 'error');
            }
            connectPeerBtn.disabled = false;
            connectPeerBtn.textContent = 'Connect';
        });
        addPeerRow.appendChild(connectPeerBtn);
        networkSection.appendChild(addPeerRow);

        container.appendChild(networkSection);
        refreshPeerList(peerListWrap);

        // ---- Debug Console (pre-release) ----
        var debugSection = Ephemera.el('div', 'settings-section');
        debugSection.appendChild(Ephemera.el('h2', '', 'Debug Console'));

        var debugPanel = Ephemera.el('div', 'debug-panel');

        // Network status card
        var debugStatus = Ephemera.el('div', 'debug-status');
        debugStatus.textContent = 'Loading debug info...';
        debugPanel.appendChild(debugStatus);

        // Log output area (scrollable monospace)
        var debugLogArea = Ephemera.el('div', 'debug-log-area');
        debugLogArea.setAttribute('role', 'log');
        debugLogArea.setAttribute('aria-label', 'Debug log output');
        debugPanel.appendChild(debugLogArea);

        // Controls row
        var debugControls = Ephemera.el('div', 'debug-controls');

        var refreshDebugBtn = Ephemera.el('button', 'btn btn-ghost btn-sm', 'Refresh');
        refreshDebugBtn.addEventListener('click', function () {
            loadDebugLog(debugStatus, debugLogArea);
        });
        debugControls.appendChild(refreshDebugBtn);

        var autoRefreshLabel = Ephemera.el('label', 'debug-auto-label');
        var autoRefreshCheck = document.createElement('input');
        autoRefreshCheck.type = 'checkbox';
        autoRefreshCheck.checked = false;
        autoRefreshLabel.appendChild(autoRefreshCheck);
        autoRefreshLabel.appendChild(document.createTextNode(' Auto-refresh'));
        debugControls.appendChild(autoRefreshLabel);

        debugPanel.appendChild(debugControls);
        debugSection.appendChild(debugPanel);
        container.appendChild(debugSection);

        // Initial load
        loadDebugLog(debugStatus, debugLogArea);

        // Auto-refresh timer
        var autoTimer = null;
        autoRefreshCheck.addEventListener('change', function () {
            if (autoRefreshCheck.checked) {
                autoTimer = setInterval(function () {
                    if (!showingSettings) {
                        clearInterval(autoTimer);
                        return;
                    }
                    loadDebugLog(debugStatus, debugLogArea);
                }, 3000);
            } else {
                if (autoTimer) clearInterval(autoTimer);
            }
        });

        // ---- Danger Zone ----
        var dangerSection = Ephemera.el('div', 'settings-section danger-zone');
        dangerSection.appendChild(Ephemera.el('h2', '', 'Danger Zone'));

        var dangerGroup = Ephemera.el('div', 'settings-group');
        dangerGroup.style.borderColor = 'rgba(255, 107, 107, 0.15)';

        var lockRow = Ephemera.el('div', 'settings-row');
        lockRow.appendChild(Ephemera.el('span', 'settings-row-label', 'Lock Identity'));
        var lockBtn = Ephemera.el('button', 'btn btn-danger btn-sm', 'Lock');
        lockBtn.addEventListener('click', async function () {
            if (!confirm('Lock your identity? You\'ll need your passphrase to unlock.')) return;
            try {
                await Ephemera.rpc('identity.lock', {});
                Ephemera.showToast('Identity locked', 'success');
                Ephemera.store.set({ identity: null, hasIdentity: false, hasKeystore: true });
                Ephemera.navigate('/unlock');
            } catch (err) {
                Ephemera.showToast('Failed: ' + err.message, 'error');
            }
        });
        lockRow.appendChild(lockBtn);
        dangerGroup.appendChild(lockRow);

        dangerSection.appendChild(dangerGroup);
        container.appendChild(dangerSection);

        // ---- About ----
        var about = Ephemera.el('div', 'version-info');
        about.innerHTML = '<strong>Ephemera</strong> v0.1.0<br>' +
            'Decentralized. Anonymous. Ephemeral.<br>' +
            'Built with Rust and conviction.';
        container.appendChild(about);
    }

    function showRecoveryPhraseModal() {
        var overlay = Ephemera.el('div', 'modal-overlay');
        overlay.setAttribute('role', 'dialog');
        overlay.setAttribute('aria-modal', 'true');

        var modal = Ephemera.el('div', 'modal-content');
        modal.appendChild(Ephemera.el('h2', '', 'Export Recovery Phrase'));
        modal.appendChild(Ephemera.el('p', '', 'Enter your passphrase to reveal your recovery phrase.'));

        var group = Ephemera.el('div', 'input-group');
        var label = Ephemera.el('label', '', 'Passphrase');
        label.setAttribute('for', 'export-passphrase');
        group.appendChild(label);

        var input = document.createElement('input');
        input.type = 'password';
        input.id = 'export-passphrase';
        input.className = 'input-field';
        input.placeholder = 'Your passphrase';
        group.appendChild(input);
        modal.appendChild(group);

        var phraseDisplay = Ephemera.el('div', 'recovery-phrase hidden');
        phraseDisplay.style.marginTop = '12px';
        modal.appendChild(phraseDisplay);

        var warningDisplay = Ephemera.el('div', 'recovery-warning hidden');
        warningDisplay.textContent = 'Store this phrase safely. Never share it.';
        modal.appendChild(warningDisplay);

        var actionsRow = Ephemera.el('div', 'modal-actions');

        var cancelBtn = Ephemera.el('button', 'btn btn-ghost', 'Close');
        cancelBtn.addEventListener('click', function () { overlay.remove(); });
        actionsRow.appendChild(cancelBtn);

        var revealBtn = Ephemera.el('button', 'btn btn-primary', 'Reveal');
        revealBtn.addEventListener('click', async function () {
            var pp = input.value;
            if (!pp) { Ephemera.showToast('Enter your passphrase', 'error'); return; }

            revealBtn.disabled = true;
            revealBtn.textContent = 'Decrypting...';

            try {
                var result = await Ephemera.rpc('identity.backup_mnemonic', { passphrase: pp });
                var phrase = result.mnemonic || result.phrase || '(Not available)';
                phraseDisplay.textContent = phrase;
                phraseDisplay.classList.remove('hidden');
                warningDisplay.classList.remove('hidden');

                revealBtn.textContent = 'Copy';
                revealBtn.disabled = false;
                revealBtn.onclick = function () {
                    if (navigator.clipboard && navigator.clipboard.writeText) {
                        navigator.clipboard.writeText(phrase).then(function () {
                            Ephemera.showToast('Copied!', 'success');
                        });
                    }
                };

                group.classList.add('hidden');
            } catch (err) {
                Ephemera.showToast('Failed: ' + err.message, 'error');
                revealBtn.disabled = false;
                revealBtn.textContent = 'Reveal';
            }
        });
        actionsRow.appendChild(revealBtn);

        modal.appendChild(actionsRow);
        overlay.appendChild(modal);

        overlay.addEventListener('click', function (e) { if (e.target === overlay) overlay.remove(); });
        function onEsc(e) { if (e.key === 'Escape') { overlay.remove(); document.removeEventListener('keydown', onEsc); } }
        document.addEventListener('keydown', onEsc);

        document.body.appendChild(overlay);
        input.focus();
    }

    function showImportQrModal(parentContainer) {
        var overlay = Ephemera.el('div', 'modal-overlay');
        overlay.setAttribute('role', 'dialog');
        overlay.setAttribute('aria-modal', 'true');

        var modal = Ephemera.el('div', 'modal-content');
        modal.appendChild(Ephemera.el('h2', '', 'Import Identity'));

        var warning = Ephemera.el('p', '');
        warning.style.color = 'var(--warning)';
        warning.textContent = 'This replaces your current identity. Make sure you\'ve backed up your recovery phrase.';
        modal.appendChild(warning);

        var group = Ephemera.el('div', 'input-group');
        var label = Ephemera.el('label', '', 'New Passphrase');
        label.setAttribute('for', 'import-passphrase');
        group.appendChild(label);

        var input = document.createElement('input');
        input.type = 'password';
        input.id = 'import-passphrase';
        input.className = 'input-field';
        input.placeholder = 'Passphrase for imported identity';
        group.appendChild(input);
        modal.appendChild(group);

        var actionsRow = Ephemera.el('div', 'modal-actions');

        var cancelBtn = Ephemera.el('button', 'btn btn-ghost', 'Cancel');
        cancelBtn.addEventListener('click', function () { overlay.remove(); });
        actionsRow.appendChild(cancelBtn);

        var scanBtn = Ephemera.el('button', 'btn btn-primary', 'Scan QR Code');
        scanBtn.addEventListener('click', async function () {
            var pp = input.value;
            if (!pp) { Ephemera.showToast('Enter a passphrase', 'error'); return; }

            overlay.remove();

            if (!Ephemera.QRScanner) {
                Ephemera.showToast('QR scanner not available', 'error');
                return;
            }

            var result = await Ephemera.QRScanner.scanAndImport(pp);

            if (result.success) {
                Ephemera.showToast('Identity imported!', 'success');
                try {
                    var profile = await Ephemera.rpc('identity.get_active');
                    Ephemera.store.set({ identity: profile, hasIdentity: true });
                } catch (_e) { /* reload */ }
                renderProfile(parentContainer);
            } else if (result.error !== 'cancelled') {
                Ephemera.showToast('Import failed: ' + result.error, 'error');
            }
        });
        actionsRow.appendChild(scanBtn);

        modal.appendChild(actionsRow);
        overlay.appendChild(modal);

        overlay.addEventListener('click', function (e) { if (e.target === overlay) overlay.remove(); });
        function onEsc(e) { if (e.key === 'Escape') { overlay.remove(); document.removeEventListener('keydown', onEsc); } }
        document.addEventListener('keydown', onEsc);

        document.body.appendChild(overlay);
        input.focus();
    }

    function formatBytes(bytes) {
        if (bytes === 0) return '0 B';
        var units = ['B', 'KB', 'MB', 'GB', 'TB'];
        var i = Math.floor(Math.log(bytes) / Math.log(1024));
        var value = bytes / Math.pow(1024, i);
        return value.toFixed(i > 0 ? 1 : 0) + ' ' + units[i];
    }

    async function renderNetworkStatus(container) {
        try {
            var status = await Ephemera.rpc('network.status', {});
            container.innerHTML = '';

            // Transport row
            var transportRow = Ephemera.el('div', 'settings-row');
            transportRow.appendChild(Ephemera.el('span', 'settings-row-label', 'Transport'));
            var transportVal = status.transport || 'none';
            var transportDisplay = transportVal === 'iroh' ? 'Iroh (QUIC + NAT traversal)'
                : transportVal === 'tcp' ? 'TCP (direct only)'
                : 'Not available';
            var transportSpan = Ephemera.el('span', 'settings-row-value', transportDisplay);
            if (transportVal === 'iroh') {
                transportSpan.style.color = 'var(--accent-green, #4caf50)';
            } else if (transportVal === 'tcp') {
                transportSpan.style.color = 'var(--accent-warn, #ff9800)';
            } else {
                transportSpan.style.color = 'var(--accent-red, #f44336)';
            }
            transportRow.appendChild(transportSpan);
            container.appendChild(transportRow);

            // Node ID row
            if (status.node_id) {
                var nodeIdRow = Ephemera.el('div', 'settings-row');
                nodeIdRow.appendChild(Ephemera.el('span', 'settings-row-label', 'Node ID'));
                var nodeIdText = status.node_id;
                if (nodeIdText.length > 20) {
                    nodeIdText = nodeIdText.slice(0, 12) + '...' + nodeIdText.slice(-8);
                }
                var nodeIdSpan = Ephemera.el('span', 'settings-row-value');
                nodeIdSpan.style.fontFamily = 'var(--font-mono, monospace)';
                nodeIdSpan.style.fontSize = 'var(--fs-xs)';
                nodeIdSpan.textContent = nodeIdText;
                nodeIdSpan.title = status.node_id;
                nodeIdRow.appendChild(nodeIdSpan);
                container.appendChild(nodeIdRow);
            }

            // Peer count row
            var peerRow = Ephemera.el('div', 'settings-row');
            peerRow.appendChild(Ephemera.el('span', 'settings-row-label', 'Connected Peers'));
            peerRow.appendChild(Ephemera.el('span', 'settings-row-value', String(status.peer_count || 0)));
            container.appendChild(peerRow);

            // Iroh availability row
            var irohRow = Ephemera.el('div', 'settings-row');
            irohRow.appendChild(Ephemera.el('span', 'settings-row-label', 'Iroh Available'));
            var irohVal = status.iroh_available ? 'Yes' : 'No';
            var irohSpan = Ephemera.el('span', 'settings-row-value', irohVal);
            irohSpan.style.color = status.iroh_available
                ? 'var(--accent-green, #4caf50)'
                : 'var(--text-tertiary)';
            irohRow.appendChild(irohSpan);
            container.appendChild(irohRow);

            // Error row (if present)
            if (status.error) {
                var errRow = Ephemera.el('div', 'settings-row');
                errRow.appendChild(Ephemera.el('span', 'settings-row-label', 'Error'));
                var errSpan = Ephemera.el('span', 'settings-row-value', status.error);
                errSpan.style.color = 'var(--accent-red, #f44336)';
                errSpan.style.fontSize = 'var(--fs-xs)';
                errRow.appendChild(errSpan);
                container.appendChild(errRow);
            }
        } catch (err) {
            container.innerHTML = '';
            var errMsg = Ephemera.el('p', '');
            errMsg.style.cssText = 'font-size:var(--fs-sm);color:var(--text-tertiary);';
            errMsg.textContent = 'Network status unavailable: ' + (err.message || err);
            container.appendChild(errMsg);
        }
    }

    async function refreshPeerList(container) {
        try {
            var result = await Ephemera.rpc('network.peers', {});
            container.innerHTML = '';
            var peers = result.peers || [];

            if (peers.length === 0) {
                var statusRow = Ephemera.el('div', '');
                statusRow.style.cssText = 'display:flex;align-items:center;gap:8px;padding:4px 0;';
                statusRow.appendChild(Ephemera.el('span', 'status-dot red'));
                statusRow.appendChild(Ephemera.el('span', '', 'No peers connected'));
                statusRow.style.fontSize = 'var(--fs-sm)';
                statusRow.style.color = 'var(--text-secondary)';
                container.appendChild(statusRow);
                return;
            }

            var statusHeader = Ephemera.el('div', '');
            statusHeader.style.cssText = 'display:flex;align-items:center;gap:8px;padding:4px 0;margin-bottom:8px;';
            statusHeader.appendChild(Ephemera.el('span', 'status-dot green'));
            statusHeader.appendChild(Ephemera.el('span', '', peers.length + ' peer' + (peers.length === 1 ? '' : 's') + ' connected'));
            statusHeader.style.fontSize = 'var(--fs-sm)';
            statusHeader.style.color = 'var(--text-secondary)';
            container.appendChild(statusHeader);

            peers.forEach(function (peer) {
                var row = Ephemera.el('div', '');
                row.style.cssText = 'display:flex;align-items:center;justify-content:space-between;padding:6px 0;border-top:1px solid var(--border);';

                var pid = peer.peer_id || '';
                var idSpan = Ephemera.el('span', '');
                idSpan.textContent = pid.length > 20 ? pid.slice(0, 12) + '...' + pid.slice(-8) : pid;
                idSpan.style.cssText = 'font-family:var(--font-mono);font-size:var(--fs-xs);color:var(--text-secondary);';
                row.appendChild(idSpan);

                var disconnectBtn = Ephemera.el('button', 'btn btn-ghost btn-sm', 'Disconnect');
                disconnectBtn.style.fontSize = 'var(--fs-xs)';
                disconnectBtn.addEventListener('click', async function () {
                    try {
                        await Ephemera.rpc('network.disconnect', { peer_id: pid });
                        Ephemera.showToast('Disconnected', 'success');
                        refreshPeerList(container);
                    } catch (err) {
                        Ephemera.showToast('Failed: ' + err.message, 'error');
                    }
                });
                row.appendChild(disconnectBtn);
                container.appendChild(row);
            });
        } catch (_err) {
            container.innerHTML = '';
            var statusRow = Ephemera.el('div', '');
            statusRow.style.cssText = 'display:flex;align-items:center;gap:8px;padding:4px 0;';
            statusRow.appendChild(Ephemera.el('span', 'status-dot amber'));
            statusRow.appendChild(Ephemera.el('span', '', 'Could not load peers'));
            statusRow.style.fontSize = 'var(--fs-sm)';
            statusRow.style.color = 'var(--text-secondary)';
            container.appendChild(statusRow);
        }
    }

    // ================================================================
    // Debug Console helpers
    // ================================================================

    async function loadDebugLog(statusEl, logAreaEl) {
        try {
            var result = await Ephemera.rpc('meta.debug_log', { count: 50 });

            // Render network status card
            statusEl.innerHTML = '';
            var ns = result.network_status || {};
            var transportLabel = ns.transport === 'iroh' ? 'Iroh (QUIC + NAT traversal)'
                : ns.transport === 'tcp' ? 'TCP (direct only)'
                : 'Not available';
            var transportColor = ns.transport === 'iroh' ? '#3dd68c'
                : ns.transport === 'tcp' ? '#ffb347'
                : '#ff6b6b';

            var statusLines = [
                { label: 'Transport', value: transportLabel, color: transportColor },
                { label: 'Node ID', value: ns.node_id ? (ns.node_id.length > 20 ? ns.node_id.slice(0, 10) + '...' : ns.node_id) : 'none' },
                { label: 'Peers', value: String(ns.peer_count || 0) },
                { label: 'Iroh Active', value: ns.iroh_active ? 'Yes' : 'No', color: ns.iroh_active ? '#3dd68c' : '#ff6b6b' },
            ];

            statusLines.forEach(function (s) {
                var row = Ephemera.el('div', 'debug-status-row');
                row.appendChild(Ephemera.el('span', 'debug-status-label', s.label + ':'));
                var val = Ephemera.el('span', 'debug-status-value', s.value);
                if (s.color) val.style.color = s.color;
                row.appendChild(val);
                statusEl.appendChild(row);
            });

            // Render log lines
            logAreaEl.innerHTML = '';
            var logs = result.logs || [];
            if (logs.length === 0) {
                logAreaEl.appendChild(Ephemera.el('div', 'debug-log-line info', 'No log entries captured yet.'));
            } else {
                logs.forEach(function (entry) {
                    var levelClass = (entry.level || '').toLowerCase();
                    var line = Ephemera.el('div', 'debug-log-line ' + levelClass);
                    var ts = Ephemera.el('span', 'debug-log-ts', entry.timestamp || '');
                    var lvl = Ephemera.el('span', 'debug-log-level', entry.level || '');
                    var tgt = Ephemera.el('span', 'debug-log-target', entry.target || '');
                    var msg = Ephemera.el('span', 'debug-log-msg', entry.message || '');
                    line.appendChild(ts);
                    line.appendChild(document.createTextNode(' '));
                    line.appendChild(lvl);
                    line.appendChild(document.createTextNode(' '));
                    line.appendChild(tgt);
                    line.appendChild(document.createTextNode(' '));
                    line.appendChild(msg);
                    logAreaEl.appendChild(line);
                });
                // Auto-scroll to bottom
                logAreaEl.scrollTop = logAreaEl.scrollHeight;
            }
        } catch (err) {
            statusEl.innerHTML = '';
            statusEl.appendChild(Ephemera.el('div', 'debug-status-row', 'Debug log unavailable: ' + (err.message || err)));
            logAreaEl.innerHTML = '';
        }
    }

    Ephemera.registerRoute('/profile', renderProfile);
})();
