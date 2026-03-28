/**
 * Ephemera -- Compose View
 *
 * Post composer with:
 * - Audience selector (Everyone / Connections / Topic)
 * - Reply context when replying to a post
 * - Circular character count indicator
 * - Media preview thumbnails
 * - Segmented TTL control
 * - Auto-growing textarea
 *
 * RPC calls:
 *   - posts.create { body, media, ttl_seconds, sensitivity, audience, parent }
 */
(function () {
    'use strict';

    var MAX_CHARS = 2000;
    var MAX_FILES = 4;
    var MAX_FILE_SIZE = 10 * 1024 * 1024; // 10 MB

    var ttlOptions = [
        { label: '1h',  seconds: 3600 },
        { label: '6h',  seconds: 21600 },
        { label: '24h', seconds: 86400 },
        { label: '7d',  seconds: 604800 },
        { label: '30d', seconds: 2592000 },
    ];

    var sensitivityOptions = [
        { key: null,       label: 'None' },
        { key: 'nudity',   label: 'Nudity' },
        { key: 'violence', label: 'Violence' },
        { key: 'spoiler',  label: 'Spoiler' },
    ];

    function renderCompose(container) {
        container.innerHTML = '';

        var state = Ephemera.store.get();
        var replyTo = state.replyTo || null;
        var selectedTtl = 86400;
        var selectedSensitivity = null;
        var selectedAudience = state.composeAudience || 'everyone';
        var selectedFiles = [];

        var wrap = Ephemera.el('div', 'compose-container');

        // Header with cancel and post button
        var header = Ephemera.el('div', 'compose-header');

        var cancelBtn = Ephemera.el('button', 'btn btn-ghost', 'Cancel');
        cancelBtn.addEventListener('click', function () {
            Ephemera.store.set({ replyTo: null });
            Ephemera.navigate(state.prevRoute || '/feed');
        });
        header.appendChild(cancelBtn);

        header.appendChild(Ephemera.el('h1', '', replyTo ? 'Reply' : 'New Post'));

        var postBtn = Ephemera.el('button', 'btn btn-primary btn-sm', replyTo ? 'Reply' : 'Post');
        postBtn.setAttribute('aria-label', 'Publish post');
        postBtn.disabled = true;
        header.appendChild(postBtn);

        wrap.appendChild(header);

        // Reply context (if replying)
        if (replyTo) {
            var replyCtx = Ephemera.el('div', 'compose-reply-context');
            var replyInfo = Ephemera.el('div', '');
            replyInfo.style.cssText = 'flex:1;min-width:0;';
            replyInfo.appendChild(Ephemera.el('div', 'reply-label', 'Replying to @' + (replyTo.author_handle || 'anon')));
            if (replyTo.body_preview) {
                replyInfo.appendChild(Ephemera.el('div', 'reply-body', replyTo.body_preview));
            }
            replyCtx.appendChild(replyInfo);

            var dismissBtn = Ephemera.el('button', 'reply-dismiss');
            dismissBtn.innerHTML = '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>';
            dismissBtn.setAttribute('aria-label', 'Cancel reply');
            dismissBtn.addEventListener('click', function () {
                Ephemera.store.set({ replyTo: null });
                replyTo = null;
                replyCtx.remove();
                header.querySelector('h1').textContent = 'New Post';
                postBtn.textContent = 'Post';
            });
            replyCtx.appendChild(dismissBtn);

            wrap.appendChild(replyCtx);
        }

        // Audience selector (shown above textarea)
        var audienceLabel = Ephemera.el('label', '', 'Audience');
        audienceLabel.style.cssText = 'display:block;font-size:13px;color:var(--text-secondary);font-weight:600;margin-bottom:6px;';
        wrap.appendChild(audienceLabel);

        var audienceRow = Ephemera.el('div', 'audience-selector');

        var audiences = [
            { key: 'everyone', label: 'Everyone', icon: '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"/><line x1="2" y1="12" x2="22" y2="12"/><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/></svg>' },
            { key: 'connections', label: 'Connections', icon: '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2"><path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/></svg>' },
            { key: 'topic', label: 'Topic...', icon: '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2"><line x1="4" y1="9" x2="20" y2="9"/><line x1="4" y1="15" x2="20" y2="15"/><line x1="10" y1="3" x2="8" y2="21"/><line x1="16" y1="3" x2="14" y2="21"/></svg>' },
        ];

        audiences.forEach(function (aud) {
            var btn = Ephemera.el('button', 'audience-option');
            btn.innerHTML = aud.icon + ' ' + aud.label;
            if (aud.key === selectedAudience || (selectedAudience.startsWith('topic:') && aud.key === 'topic')) {
                btn.classList.add('active');
            }
            btn.addEventListener('click', function () {
                if (aud.key === 'topic') {
                    showTopicPicker(function (topic) {
                        if (topic) {
                            selectedAudience = 'topic:' + topic;
                            updateAudienceUI();
                        }
                    });
                } else {
                    selectedAudience = aud.key;
                    updateAudienceUI();
                }
            });
            audienceRow.appendChild(btn);
        });

        function updateAudienceUI() {
            var buttons = audienceRow.querySelectorAll('.audience-option');
            buttons.forEach(function (b, i) {
                b.classList.remove('active');
                if (audiences[i].key === selectedAudience) b.classList.add('active');
                if (audiences[i].key === 'topic' && selectedAudience.startsWith('topic:')) {
                    b.classList.add('active');
                    b.innerHTML = audiences[i].icon + ' #' + selectedAudience.slice(6);
                }
            });
            Ephemera.store.set({ composeAudience: selectedAudience });
        }

        wrap.appendChild(audienceRow);

        // Compose row: avatar + textarea
        var composeRow = Ephemera.el('div', 'compose-row');

        var identity = state.identity || {};
        var userName = identity.display_name || 'You';
        composeRow.appendChild(Ephemera.avatar(userName, null, identity.avatar_url || null));

        var textWrap = Ephemera.el('div', '');
        textWrap.style.cssText = 'flex:1;min-width:0;';

        var textarea = document.createElement('textarea');
        textarea.className = 'compose-textarea';
        textarea.placeholder = replyTo
            ? 'Write your reply...'
            : 'What\'s on your mind? It won\'t last forever...';
        textarea.maxLength = MAX_CHARS;
        textarea.rows = 4;
        textarea.setAttribute('aria-label', 'Post content');
        textWrap.appendChild(textarea);

        composeRow.appendChild(textWrap);
        wrap.appendChild(composeRow);

        // Media preview grid
        var mediaPreviewGrid = Ephemera.el('div', 'media-preview-grid');
        wrap.appendChild(mediaPreviewGrid);

        // Toolbar: media buttons, char counter
        var toolbar = Ephemera.el('div', 'compose-toolbar');

        // Hidden file input
        var fileInput = document.createElement('input');
        fileInput.type = 'file';
        fileInput.accept = 'image/*,video/*';
        fileInput.multiple = true;
        fileInput.style.display = 'none';
        fileInput.setAttribute('aria-label', 'Choose media files');
        wrap.appendChild(fileInput);

        var mediaBtns = Ephemera.el('div', 'compose-media-btns');

        var photoBtn = Ephemera.el('button', 'compose-media-btn');
        photoBtn.innerHTML = '<svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2"><rect x="3" y="3" width="18" height="18" rx="2" ry="2"/><circle cx="8.5" cy="8.5" r="1.5"/><polyline points="21 15 16 10 5 21"/></svg>';
        photoBtn.title = 'Add photo';
        photoBtn.setAttribute('aria-label', 'Add photo');
        photoBtn.addEventListener('click', function () {
            fileInput.accept = 'image/*';
            fileInput.click();
        });
        mediaBtns.appendChild(photoBtn);

        var videoBtn = Ephemera.el('button', 'compose-media-btn');
        videoBtn.innerHTML = '<svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2"><polygon points="23 7 16 12 23 17 23 7"/><rect x="1" y="5" width="15" height="14" rx="2" ry="2"/></svg>';
        videoBtn.title = 'Add video';
        videoBtn.setAttribute('aria-label', 'Add video');
        videoBtn.addEventListener('click', function () {
            fileInput.accept = 'video/*';
            fileInput.click();
        });
        mediaBtns.appendChild(videoBtn);

        toolbar.appendChild(mediaBtns);

        var spacer = Ephemera.el('div', '');
        spacer.style.flex = '1';
        toolbar.appendChild(spacer);

        // Circular character counter
        var charRing = Ephemera.el('div', 'char-counter-ring');
        charRing.innerHTML =
            '<svg viewBox="0 0 28 28">' +
            '<circle class="ring-bg" cx="14" cy="14" r="12"/>' +
            '<circle class="ring-fill" cx="14" cy="14" r="12"/>' +
            '</svg>';
        toolbar.appendChild(charRing);

        wrap.appendChild(toolbar);

        // TTL selector
        var ttlLabel = Ephemera.el('label', '', 'Expires after');
        ttlLabel.style.cssText = 'display:block;font-size:13px;color:var(--text-secondary);font-weight:600;margin-top:4px;margin-bottom:4px;';
        wrap.appendChild(ttlLabel);

        var ttlRow = Ephemera.el('div', 'ttl-selector');
        ttlRow.setAttribute('role', 'radiogroup');
        ttlRow.setAttribute('aria-label', 'Post expiry time');
        ttlOptions.forEach(function (opt) {
            var btn = Ephemera.el('button', 'ttl-option', opt.label);
            btn.setAttribute('role', 'radio');
            btn.setAttribute('aria-checked', opt.seconds === selectedTtl ? 'true' : 'false');
            if (opt.seconds === selectedTtl) btn.classList.add('active');
            btn.addEventListener('click', function () {
                selectedTtl = opt.seconds;
                ttlRow.querySelectorAll('.ttl-option').forEach(function (b) {
                    b.classList.remove('active');
                    b.setAttribute('aria-checked', 'false');
                });
                btn.classList.add('active');
                btn.setAttribute('aria-checked', 'true');
            });
            ttlRow.appendChild(btn);
        });
        wrap.appendChild(ttlRow);

        // Sensitivity labels
        var sensLabel = Ephemera.el('label', '', 'Content sensitivity');
        sensLabel.style.cssText = 'display:block;font-size:13px;color:var(--text-secondary);font-weight:600;margin-top:12px;margin-bottom:4px;';
        wrap.appendChild(sensLabel);

        var sensRow = Ephemera.el('div', 'sensitivity-selector');
        sensRow.setAttribute('role', 'radiogroup');
        sensRow.setAttribute('aria-label', 'Content sensitivity');
        sensitivityOptions.forEach(function (opt) {
            var btn = Ephemera.el('button', 'sensitivity-option', opt.label);
            btn.setAttribute('role', 'radio');
            btn.setAttribute('aria-checked', opt.key === selectedSensitivity ? 'true' : 'false');
            if (opt.key === selectedSensitivity) btn.classList.add('active');
            if (opt.key === 'spoiler') btn.classList.add('spoiler');
            btn.addEventListener('click', function () {
                selectedSensitivity = opt.key;
                sensRow.querySelectorAll('.sensitivity-option').forEach(function (b) {
                    b.classList.remove('active');
                    b.setAttribute('aria-checked', 'false');
                });
                btn.classList.add('active');
                btn.setAttribute('aria-checked', 'true');
            });
            sensRow.appendChild(btn);
        });
        wrap.appendChild(sensRow);

        // Upload progress bar
        var progressWrap = Ephemera.el('div', 'upload-progress hidden');
        var progressBar = Ephemera.el('div', 'upload-progress-bar');
        var progressFill = Ephemera.el('div', 'upload-progress-fill');
        progressFill.style.width = '0%';
        progressBar.appendChild(progressFill);
        progressWrap.appendChild(progressBar);
        wrap.appendChild(progressWrap);

        container.appendChild(wrap);

        // ---- Event handlers ----

        var CIRCUMFERENCE = 2 * Math.PI * 12;
        var ringFill = charRing.querySelector('.ring-fill');

        function updateCharCounter() {
            var len = textarea.value.length;
            var ratio = Math.min(len / MAX_CHARS, 1);
            var offset = CIRCUMFERENCE - (ratio * CIRCUMFERENCE);
            ringFill.style.strokeDashoffset = offset;

            charRing.classList.remove('near-limit', 'over-limit');
            if (len > MAX_CHARS * 0.9) charRing.classList.add('near-limit');
            if (len >= MAX_CHARS) charRing.classList.add('over-limit');

            var hasContent = len > 0 || selectedFiles.length > 0;
            postBtn.disabled = !hasContent;
            if (hasContent) {
                postBtn.classList.add('pulse');
            } else {
                postBtn.classList.remove('pulse');
            }
        }

        // Mention autocomplete dropdown
        var mentionDropdown = Ephemera.el('div', 'mention-dropdown hidden');
        textWrap.style.position = 'relative';
        textWrap.appendChild(mentionDropdown);

        var mentionCache = null;

        async function loadConnectionsForMentions() {
            if (mentionCache) return mentionCache;
            try {
                var result = await Ephemera.rpc('social.list_connections', { status: 'connected' });
                mentionCache = (result.connections || []).map(function (c) {
                    return {
                        handle: c.handle || null,
                        display_name: c.display_name || 'Anonymous',
                        pubkey: c.pseudonym_id,
                    };
                });
            } catch (_e) {
                mentionCache = [];
            }
            return mentionCache;
        }

        function getMentionQuery() {
            var val = textarea.value;
            var pos = textarea.selectionStart || 0;
            if (pos === 0) return null;
            // Walk backwards to find @
            var i = pos - 1;
            while (i >= 0 && /[a-zA-Z0-9_-]/.test(val[i])) i--;
            if (i >= 0 && val[i] === '@') {
                // Only trigger if @ is at start or preceded by whitespace/newline
                if (i === 0 || /\s/.test(val[i - 1])) {
                    return { start: i, query: val.slice(i + 1, pos) };
                }
            }
            return null;
        }

        async function updateMentionDropdown() {
            var mq = getMentionQuery();
            if (!mq) {
                mentionDropdown.classList.add('hidden');
                return;
            }
            var connections = await loadConnectionsForMentions();
            var q = mq.query.toLowerCase();
            var matches;
            if (q.length === 0) {
                // Show all connections when user just typed '@'
                matches = connections.slice(0, 6);
            } else {
                matches = connections.filter(function (c) {
                    return (c.handle && c.handle.toLowerCase().indexOf(q) !== -1) ||
                           c.display_name.toLowerCase().indexOf(q) !== -1;
                }).slice(0, 5);
            }

            mentionDropdown.innerHTML = '';
            if (matches.length === 0) {
                mentionDropdown.classList.add('hidden');
                return;
            }

            matches.forEach(function (m) {
                var item = Ephemera.el('div', 'mention-item');
                var label = m.display_name;
                if (m.handle) label += ' (@' + m.handle + ')';
                item.textContent = label;
                item.addEventListener('mousedown', function (e) {
                    e.preventDefault(); // prevent blur
                    var val = textarea.value;
                    var replacement = m.handle || m.pubkey.slice(0, 8);
                    var before = val.slice(0, mq.start + 1); // include @
                    var after = val.slice(textarea.selectionStart || 0);
                    textarea.value = before + replacement + ' ' + after;
                    var newPos = mq.start + 1 + replacement.length + 1;
                    textarea.setSelectionRange(newPos, newPos);
                    mentionDropdown.classList.add('hidden');
                    updateCharCounter();
                    textarea.focus();
                });
                mentionDropdown.appendChild(item);
            });
            mentionDropdown.classList.remove('hidden');
        }

        textarea.addEventListener('input', function () {
            updateCharCounter();
            textarea.style.height = 'auto';
            textarea.style.height = Math.min(textarea.scrollHeight, 300) + 'px';
            updateMentionDropdown();
        });

        textarea.addEventListener('blur', function () {
            // Delay so mousedown on dropdown item fires first
            setTimeout(function () { mentionDropdown.classList.add('hidden'); }, 150);
        });

        // File handling
        fileInput.addEventListener('change', function () {
            if (!fileInput.files) return;
            Array.from(fileInput.files).forEach(function (file) {
                if (selectedFiles.length >= MAX_FILES) {
                    Ephemera.showToast('Maximum ' + MAX_FILES + ' files', 'error');
                    return;
                }
                if (file.size > MAX_FILE_SIZE) {
                    Ephemera.showToast(file.name + ' exceeds 10 MB', 'error');
                    return;
                }
                selectedFiles.push({ file: file, dataHex: null });
                renderMediaPreviews();
                updateCharCounter();
            });
            fileInput.value = '';
        });

        function renderMediaPreviews() {
            mediaPreviewGrid.innerHTML = '';
            selectedFiles.forEach(function (entry, index) {
                var item = Ephemera.el('div', 'media-preview-item');

                if (entry.file.type.startsWith('image/')) {
                    var img = document.createElement('img');
                    img.alt = entry.file.name;
                    var url = URL.createObjectURL(entry.file);
                    img.src = url;
                    img.onload = function () { URL.revokeObjectURL(url); };
                    item.appendChild(img);
                } else {
                    var placeholder = Ephemera.el('div', '');
                    placeholder.style.cssText = 'width:100%;height:100%;display:flex;align-items:center;justify-content:center;color:var(--text-tertiary);font-size:11px;text-align:center;padding:8px;';
                    placeholder.textContent = entry.file.name;
                    item.appendChild(placeholder);
                }

                var removeBtn = Ephemera.el('button', 'remove-media');
                removeBtn.textContent = '\u00d7';
                removeBtn.setAttribute('aria-label', 'Remove ' + entry.file.name);
                removeBtn.addEventListener('click', function () {
                    selectedFiles.splice(index, 1);
                    renderMediaPreviews();
                    updateCharCounter();
                });
                item.appendChild(removeBtn);

                mediaPreviewGrid.appendChild(item);
            });
        }

        // Post submission
        postBtn.addEventListener('click', async function () {
            var body = textarea.value.trim();
            if (!body && selectedFiles.length === 0) {
                Ephemera.showToast('Write something or add media', 'error');
                return;
            }

            postBtn.disabled = true;
            postBtn.classList.remove('pulse');
            postBtn.textContent = 'Posting...';

            // Encode media files to hex
            var mediaPayload = [];
            if (selectedFiles.length > 0) {
                progressWrap.classList.remove('hidden');
                for (var i = 0; i < selectedFiles.length; i++) {
                    progressFill.style.width = ((i / selectedFiles.length) * 100).toFixed(0) + '%';
                    try {
                        var hexData = await readFileAsHex(selectedFiles[i].file);
                        mediaPayload.push({
                            data_hex: hexData,
                            filename: selectedFiles[i].file.name,
                        });
                    } catch (err) {
                        Ephemera.showToast('Failed to read ' + selectedFiles[i].file.name, 'error');
                        postBtn.disabled = false;
                        postBtn.textContent = replyTo ? 'Reply' : 'Post';
                        progressWrap.classList.add('hidden');
                        return;
                    }
                }
                progressFill.style.width = '100%';
            }

            try {
                var params = {
                    body: body,
                    ttl_seconds: selectedTtl,
                };
                if (mediaPayload.length > 0) params.media = mediaPayload;
                if (selectedSensitivity) params.sensitivity = selectedSensitivity;
                if (selectedAudience !== 'everyone') params.audience = selectedAudience;

                // Include parent hash if replying
                if (replyTo && replyTo.content_hash) {
                    params.parent = replyTo.content_hash;
                }

                await Ephemera.rpc('posts.create', params);
                Ephemera.store.set({ replyTo: null, composeAudience: 'everyone' });
                Ephemera.showToast(replyTo ? 'Reply posted!' : 'Posted!', 'success');
                Ephemera.navigate('/feed');
            } catch (err) {
                console.error('Post creation failed:', err);
                Ephemera.showToast('Failed: ' + err.message, 'error');
                postBtn.disabled = false;
                postBtn.textContent = replyTo ? 'Reply' : 'Post';
                progressWrap.classList.add('hidden');
            }
        });

        textarea.focus();
    }

    function showTopicPicker(callback) {
        var overlay = Ephemera.el('div', 'modal-overlay');
        overlay.setAttribute('role', 'dialog');
        overlay.setAttribute('aria-modal', 'true');

        var modal = Ephemera.el('div', 'modal-content');
        modal.appendChild(Ephemera.el('h2', '', 'Choose a Topic'));
        modal.appendChild(Ephemera.el('p', '', 'Posts tagged with a topic are visible to anyone browsing that topic.'));

        var group = Ephemera.el('div', 'input-group');
        var input = document.createElement('input');
        input.type = 'text';
        input.className = 'input-field';
        input.placeholder = 'Enter topic name...';
        input.maxLength = 40;
        input.setAttribute('aria-label', 'Topic name');
        group.appendChild(input);
        modal.appendChild(group);

        var suggestedTopics = ['general', 'tech', 'music', 'art', 'gaming', 'politics', 'memes', 'science'];
        var suggestions = Ephemera.el('div', '');
        suggestions.style.cssText = 'display:flex;flex-wrap:wrap;gap:8px;margin-bottom:16px;';
        suggestedTopics.forEach(function (t) {
            var pill = Ephemera.el('button', 'audience-option', '#' + t);
            pill.addEventListener('click', function () {
                input.value = t;
            });
            suggestions.appendChild(pill);
        });
        modal.appendChild(suggestions);

        var actionsRow = Ephemera.el('div', 'modal-actions');
        var cancelBtn = Ephemera.el('button', 'btn btn-ghost', 'Cancel');
        cancelBtn.addEventListener('click', function () {
            overlay.remove();
            callback(null);
        });
        actionsRow.appendChild(cancelBtn);

        var selectBtn = Ephemera.el('button', 'btn btn-primary', 'Select');
        selectBtn.addEventListener('click', function () {
            var val = input.value.trim().toLowerCase().replace(/[^a-z0-9_-]/g, '');
            if (!val) {
                Ephemera.showToast('Enter a topic name', 'error');
                return;
            }
            overlay.remove();
            callback(val);
        });
        actionsRow.appendChild(selectBtn);

        modal.appendChild(actionsRow);
        overlay.appendChild(modal);

        overlay.addEventListener('click', function (e) {
            if (e.target === overlay) { overlay.remove(); callback(null); }
        });

        document.body.appendChild(overlay);
        input.focus();
    }

    function readFileAsHex(file) {
        return new Promise(function (resolve, reject) {
            var reader = new FileReader();
            reader.onload = function () {
                var arr = new Uint8Array(reader.result);
                var hex = '';
                for (var i = 0; i < arr.length; i++) {
                    hex += arr[i].toString(16).padStart(2, '0');
                }
                resolve(hex);
            };
            reader.onerror = function () { reject(new Error('Failed to read file')); };
            reader.readAsArrayBuffer(file);
        });
    }

    Ephemera.registerRoute('/compose', renderCompose);
})();
