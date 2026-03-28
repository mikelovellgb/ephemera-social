/**
 * Ephemera -- Messages View
 *
 * Unified messaging:
 * - 1:1 encrypted conversations + group chats in one list
 * - Group chat icon (multi-person) vs 1:1 icon (single person)
 * - Group name shown instead of peer name for group chats
 * - "New Group Chat" button -> select people to add
 * - Chat bubbles with sent/delivered/read indicators
 * - Sender names in group chat bubbles
 * - Auto-growing textarea input
 * - Encryption badge for DMs
 *
 * RPC calls:
 *   - messages.list_conversations {}
 *   - messages.get_thread { conversation_id, limit }
 *   - messages.send { recipient, body }
 *   - messages.mark_read { conversation_id }
 *   - group_chats.list {}
 *   - group_chats.create_private { name, members }
 *   - group_chats.send { chat_id, body }
 *   - group_chats.messages { chat_id, limit }
 *   - group_chats.leave { chat_id }
 *   - social.list_connections { status }
 */
(function () {
    'use strict';

    var currentConversation = null;

    // ================================================================
    // SVG Icons
    // ================================================================

    var ICON_DM = '<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2"><path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2"/><circle cx="12" cy="7" r="4"/></svg>';
    var ICON_GROUP_CHAT = '<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2"><path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M23 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/></svg>';

    // ================================================================
    // Conversation Item (unified for DMs + group chats)
    // ================================================================

    function renderConversationItem(conv) {
        var isGC = conv._isGroupChat;
        var item = Ephemera.el('div', 'conversation-item');
        item.setAttribute('role', 'button');
        item.setAttribute('tabindex', '0');

        var displayName = isGC
            ? (conv.name || 'Group Chat')
            : (conv.peer_display_name || conv.name || 'Anonymous');

        item.setAttribute('aria-label',
            displayName + (conv.unread_count ? ', ' + conv.unread_count + ' unread' : ''));
        if (conv.unread_count > 0) item.classList.add('unread');

        // Avatar with type indicator
        var avatarWrap = Ephemera.el('div', 'conv-avatar-wrap');
        avatarWrap.appendChild(Ephemera.avatar(displayName, 'avatar-md',
            isGC ? null : (conv.peer_avatar_url || null)));
        var typeIcon = Ephemera.el('div', 'conv-type-icon' + (isGC ? ' conv-type-gc' : ''));
        typeIcon.innerHTML = isGC ? ICON_GROUP_CHAT : ICON_DM;
        typeIcon.title = isGC ? 'Group chat' : 'Direct message';
        avatarWrap.appendChild(typeIcon);
        item.appendChild(avatarWrap);

        var info = Ephemera.el('div', 'conversation-info');

        var nameRow = Ephemera.el('div', 'conversation-name');
        nameRow.appendChild(document.createTextNode(displayName));
        if (!isGC && conv.peer_handle) {
            var handleEl = Ephemera.el('span', 'post-author-handle', ' @' + conv.peer_handle);
            handleEl.style.marginLeft = '4px';
            nameRow.appendChild(handleEl);
        }
        if (isGC) {
            var countEl = Ephemera.el('span', 'conv-member-count');
            countEl.textContent = (conv.member_count || '?') + ' people';
            nameRow.appendChild(countEl);
        }
        if (conv.unread_count > 0) {
            nameRow.appendChild(Ephemera.el('span', 'unread-badge', String(conv.unread_count)));
        }
        info.appendChild(nameRow);

        var preview = Ephemera.el('div', 'conversation-preview',
            conv.last_message_preview || conv.last_message || 'No messages yet');
        info.appendChild(preview);
        item.appendChild(info);

        if (conv.last_message_at) {
            var lastMsgMs = conv.last_message_at;
            if (lastMsgMs && lastMsgMs < 1e12) lastMsgMs = lastMsgMs * 1000;
            item.appendChild(Ephemera.el('div', 'conversation-time',
                Ephemera.timeAgo(lastMsgMs)));
        }

        function openConv() {
            currentConversation = conv;
            var container = document.getElementById('main-content');
            if (isGC) {
                renderGroupChatView(container);
            } else {
                renderChatView(container);
            }
        }
        item.addEventListener('click', openConv);
        item.addEventListener('keydown', function (e) {
            if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); openConv(); }
        });

        return item;
    }

    // ================================================================
    // New Group Chat Modal
    // ================================================================

    function showNewGroupChatModal(onCreated) {
        var overlay = Ephemera.el('div', 'modal-overlay');
        overlay.setAttribute('role', 'dialog');
        overlay.setAttribute('aria-modal', 'true');

        var modal = Ephemera.el('div', 'modal-content');
        modal.appendChild(Ephemera.el('h2', '', 'New Group Chat'));
        modal.appendChild(Ephemera.el('p', '', 'Pick connections to add to the conversation.'));

        // Chat name
        var nameGroup = Ephemera.el('div', 'input-group');
        nameGroup.appendChild(Ephemera.el('label', '', 'Chat Name (optional)'));
        var nameInput = document.createElement('input');
        nameInput.type = 'text';
        nameInput.className = 'input-field';
        nameInput.placeholder = 'Weekend Plans';
        nameInput.maxLength = 50;
        nameGroup.appendChild(nameInput);
        modal.appendChild(nameGroup);

        // Connection picker
        var connectionsArea = Ephemera.el('div', 'gc-connections-area');
        connectionsArea.appendChild(Ephemera.el('div', 'spinner'));
        modal.appendChild(connectionsArea);

        var selectedMembers = [];

        Ephemera.rpc('social.list_connections', { status: 'connected' }).then(function (result) {
            connectionsArea.innerHTML = '';
            var connections = result.connections || [];
            if (connections.length === 0) {
                connectionsArea.appendChild(Ephemera.el('p', '', 'No connections yet. Add people from Discover first.'));
                return;
            }
            connections.forEach(function (conn) {
                var row = Ephemera.el('label', 'gc-connection-row');
                var checkbox = document.createElement('input');
                checkbox.type = 'checkbox';
                checkbox.className = 'gc-checkbox';
                checkbox.value = conn.pseudonym_id;
                checkbox.addEventListener('change', function () {
                    if (checkbox.checked) {
                        selectedMembers.push(conn.pseudonym_id);
                    } else {
                        selectedMembers = selectedMembers.filter(function (id) {
                            return id !== conn.pseudonym_id;
                        });
                    }
                });
                row.appendChild(checkbox);
                row.appendChild(Ephemera.avatar(conn.display_name || 'A', 'avatar-sm'));
                var label = conn.display_name || 'Anonymous';
                if (conn.handle) label += ' (@' + conn.handle + ')';
                row.appendChild(Ephemera.el('span', 'gc-connection-name', label));
                connectionsArea.appendChild(row);
            });
        }).catch(function () {
            connectionsArea.innerHTML = '';
            connectionsArea.appendChild(Ephemera.el('p', '', 'Could not load connections.'));
        });

        var actionsRow = Ephemera.el('div', 'modal-actions');

        var cancelBtn = Ephemera.el('button', 'btn btn-ghost', 'Cancel');
        cancelBtn.addEventListener('click', function () { overlay.remove(); });
        actionsRow.appendChild(cancelBtn);

        var createBtn = Ephemera.el('button', 'btn btn-primary', 'Create Chat');
        createBtn.addEventListener('click', async function () {
            if (selectedMembers.length === 0) {
                Ephemera.showToast('Select at least one person', 'error');
                return;
            }
            createBtn.disabled = true;
            createBtn.textContent = 'Creating...';
            try {
                await Ephemera.rpc('group_chats.create_private', {
                    name: nameInput.value.trim() || undefined,
                    members: selectedMembers,
                });
                Ephemera.showToast('Group chat created!', 'success');
                overlay.remove();
                if (onCreated) onCreated();
            } catch (err) {
                Ephemera.showToast('Failed: ' + err.message, 'error');
                createBtn.disabled = false;
                createBtn.textContent = 'Create Chat';
            }
        });
        actionsRow.appendChild(createBtn);
        modal.appendChild(actionsRow);

        overlay.appendChild(modal);
        overlay.addEventListener('click', function (e) { if (e.target === overlay) overlay.remove(); });
        function onEsc(e) {
            if (e.key === 'Escape') { overlay.remove(); document.removeEventListener('keydown', onEsc); }
        }
        document.addEventListener('keydown', onEsc);
        document.body.appendChild(overlay);
        nameInput.focus();
    }

    // ================================================================
    // Conversation List (unified DMs + group chats)
    // ================================================================

    async function renderConversationList(container) {
        container.innerHTML = '';

        var header = Ephemera.el('div', 'messages-header');
        header.appendChild(Ephemera.el('h1', '', 'Messages'));

        var newGcBtn = Ephemera.el('button', 'btn btn-ghost btn-sm');
        newGcBtn.innerHTML = '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2"><path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><line x1="20" y1="8" x2="20" y2="14"/><line x1="23" y1="11" x2="17" y2="11"/></svg>';
        newGcBtn.setAttribute('aria-label', 'New group chat');
        newGcBtn.title = 'New Group Chat';
        newGcBtn.addEventListener('click', function () {
            showNewGroupChatModal(function () { renderConversationList(container); });
        });
        header.appendChild(newGcBtn);
        container.appendChild(header);

        // Loading
        var loading = Ephemera.el('div', 'loading-state');
        loading.setAttribute('role', 'status');
        loading.appendChild(Ephemera.el('div', 'spinner'));
        loading.appendChild(Ephemera.el('p', '', 'Loading conversations...'));
        container.appendChild(loading);

        try {
            // Load DMs and group chats in parallel
            var dmPromise = Ephemera.rpc('messages.list_conversations', {});
            var gcPromise = Ephemera.rpc('group_chats.list').catch(function () { return { chats: [] }; });
            var results = await Promise.all([dmPromise, gcPromise]);

            container.removeChild(loading);

            var dmConversations = results[0].conversations || results[0] || [];
            var groupChats = results[1].chats || results[1].conversations || [];

            // Normalize all conversations
            var allConversations = [];
            if (Array.isArray(dmConversations)) {
                dmConversations.forEach(function (c) {
                    allConversations.push(Object.assign({}, c, { _isGroupChat: false }));
                });
            }
            if (Array.isArray(groupChats)) {
                groupChats.forEach(function (gc) {
                    allConversations.push(Object.assign({}, gc, { _isGroupChat: true }));
                });
            }

            // Sort by most recent activity
            allConversations.sort(function (a, b) {
                var aTime = a.last_message_at || 0;
                var bTime = b.last_message_at || 0;
                return bTime - aTime;
            });

            if (allConversations.length === 0) {
                var empty = Ephemera.el('div', 'empty-state');
                empty.setAttribute('role', 'status');

                var icon = Ephemera.el('div', 'empty-state-icon');
                icon.innerHTML = '<svg viewBox="0 0 24 24" width="64" height="64" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>';
                empty.appendChild(icon);

                empty.appendChild(Ephemera.el('h3', '', 'Your conversations will appear here'));
                empty.appendChild(Ephemera.el('p', '',
                    'Connect with someone from Discover, or start a group chat.'));

                var row = Ephemera.el('div', '');
                row.style.cssText = 'display:flex;gap:12px;justify-content:center;flex-wrap:wrap;';

                var discoverBtn = Ephemera.el('button', 'btn btn-primary', 'Find People');
                discoverBtn.addEventListener('click', function () { Ephemera.navigate('/discover'); });
                row.appendChild(discoverBtn);

                var gcBtn = Ephemera.el('button', 'btn btn-secondary', 'New Group Chat');
                gcBtn.addEventListener('click', function () {
                    showNewGroupChatModal(function () { renderConversationList(container); });
                });
                row.appendChild(gcBtn);

                empty.appendChild(row);
                container.appendChild(empty);
                return;
            }

            var list = Ephemera.el('div', 'conversation-list');
            list.setAttribute('role', 'list');
            allConversations.forEach(function (conv) {
                list.appendChild(renderConversationItem(conv));
            });
            container.appendChild(list);

        } catch (err) {
            container.removeChild(loading);
            console.error('Messages load error:', err);

            var empty = Ephemera.el('div', 'empty-state');
            empty.setAttribute('role', 'status');
            var icon = Ephemera.el('div', 'empty-state-icon');
            icon.innerHTML = '<svg viewBox="0 0 24 24" width="64" height="64" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>';
            empty.appendChild(icon);
            empty.appendChild(Ephemera.el('h3', '', 'Your conversations will appear here'));
            empty.appendChild(Ephemera.el('p', '', 'Connect with someone to start messaging.'));
            container.appendChild(empty);
        }
    }

    // ================================================================
    // 1:1 Chat View
    // ================================================================

    async function renderChatView(container) {
        container.innerHTML = '';

        var conv = currentConversation;
        if (!conv) { renderConversationList(container); return; }

        var chatView = Ephemera.el('div', 'chat-view');

        // Header
        var header = Ephemera.el('div', 'chat-header');
        var backBtn = Ephemera.el('button', 'chat-back');
        backBtn.innerHTML = '<svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2"><polyline points="15 18 9 12 15 6"/></svg>';
        backBtn.setAttribute('aria-label', 'Back to conversations');
        backBtn.addEventListener('click', function () {
            currentConversation = null;
            renderConversationList(container);
        });
        header.appendChild(backBtn);

        var displayName = conv.peer_display_name || conv.name || 'Anonymous';
        header.appendChild(Ephemera.avatar(displayName, 'avatar-sm', conv.peer_avatar_url || null));

        var headerInfo = Ephemera.el('div', 'chat-header-info');
        headerInfo.appendChild(Ephemera.el('div', 'chat-header-name', displayName));
        var encBadge = Ephemera.el('div', 'encryption-badge');
        encBadge.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="3" y="11" width="18" height="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/></svg>';
        encBadge.appendChild(document.createTextNode(' End-to-end encrypted'));
        headerInfo.appendChild(encBadge);
        header.appendChild(headerInfo);
        chatView.appendChild(header);

        // Messages
        var messagesArea = Ephemera.el('div', 'chat-messages');
        messagesArea.setAttribute('role', 'log');
        messagesArea.setAttribute('aria-label', 'Messages with ' + displayName);

        var convId = conv.conversation_id || conv.peer || conv.peer_key || conv.public_key || conv.pseudonym_id || '';
        var recipientId = conv.peer || conv.their_pubkey || conv.peer_key || conv.public_key || conv.pseudonym_id || '';

        var loadingMsg = Ephemera.el('div', 'loading-state');
        loadingMsg.style.padding = '40px';
        loadingMsg.appendChild(Ephemera.el('div', 'spinner'));
        messagesArea.appendChild(loadingMsg);
        chatView.appendChild(messagesArea);

        loadDmMessages(messagesArea, convId, loadingMsg);

        if (conv.unread_count > 0 && convId) {
            Ephemera.rpc('messages.mark_read', { conversation_id: convId }).catch(function () {});
        }

        // Compose
        var compose = Ephemera.el('div', 'chat-compose');
        var inputWrap = Ephemera.el('div', 'chat-input-wrap');
        var input = document.createElement('textarea');
        input.className = 'chat-input';
        input.placeholder = 'Message...';
        input.rows = 1;
        input.setAttribute('aria-label', 'Type a message');
        inputWrap.appendChild(input);
        compose.appendChild(inputWrap);

        var sendBtn = Ephemera.el('button', 'chat-send-btn');
        sendBtn.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><line x1="22" y1="2" x2="11" y2="13"/><polygon points="22 2 15 22 11 13 2 9 22 2"/></svg>';
        sendBtn.setAttribute('aria-label', 'Send message');

        input.addEventListener('input', function () {
            input.style.height = 'auto';
            input.style.height = Math.min(input.scrollHeight, 120) + 'px';
        });

        async function doSend() {
            var body = input.value.trim();
            if (!body) return;
            sendBtn.disabled = true;
            try {
                await Ephemera.rpc('messages.send', { recipient: recipientId, body: body });
                var bubble = Ephemera.el('div', 'message-bubble sent');
                bubble.appendChild(document.createTextNode(body));
                bubble.appendChild(Ephemera.el('div', 'message-time', 'now'));
                bubble.appendChild(Ephemera.el('div', 'message-status', 'Sent'));
                messagesArea.appendChild(bubble);
                messagesArea.scrollTop = messagesArea.scrollHeight;
                input.value = '';
                input.style.height = 'auto';
            } catch (err) {
                Ephemera.showToast('Failed to send: ' + err.message, 'error');
            }
            sendBtn.disabled = false;
            input.focus();
        }

        sendBtn.addEventListener('click', doSend);
        input.addEventListener('keydown', function (e) {
            if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); doSend(); }
        });
        compose.appendChild(sendBtn);
        chatView.appendChild(compose);

        container.appendChild(chatView);
        input.focus();
    }

    async function loadDmMessages(messagesArea, convId, loadingEl) {
        try {
            var result = await Ephemera.rpc('messages.get_thread', {
                conversation_id: convId,
                limit: 50,
            });
            if (loadingEl && loadingEl.parentNode) loadingEl.remove();

            var messages = result.messages || result || [];

            if (!Array.isArray(messages) || messages.length === 0) {
                var empty = Ephemera.el('div', 'empty-state');
                empty.style.padding = '40px 20px';
                empty.appendChild(Ephemera.el('p', '', 'No messages yet. Say hello!'));
                messagesArea.appendChild(empty);
            } else {
                messages.forEach(function (msg) {
                    messagesArea.appendChild(renderDmBubble(msg));
                });
                messagesArea.scrollTop = messagesArea.scrollHeight;
            }
        } catch (err) {
            if (loadingEl && loadingEl.parentNode) loadingEl.remove();
            var empty = Ephemera.el('div', 'empty-state');
            empty.style.padding = '40px 20px';
            var msg = (err && err.message) ? err.message : '';
            if (msg.indexOf('not found') !== -1 || msg.indexOf('no thread') !== -1) {
                empty.appendChild(Ephemera.el('p', '', 'No messages yet. Say hello!'));
            } else {
                empty.appendChild(Ephemera.el('p', '', 'Could not load messages. Try again later.'));
            }
            messagesArea.appendChild(empty);
        }
    }

    function renderDmBubble(msg) {
        var bubble = Ephemera.el('div', 'message-bubble');
        bubble.classList.add(msg.is_own ? 'sent' : 'received');
        bubble.appendChild(document.createTextNode(msg.body || ''));

        if (msg.created_at) {
            var msgMs = msg.created_at;
            if (msgMs && msgMs < 1e12) msgMs = msgMs * 1000;
            bubble.appendChild(Ephemera.el('div', 'message-time', Ephemera.timeAgo(msgMs)));
        }

        if (msg.is_own && msg.status) {
            var statusText = '';
            switch (msg.status) {
                case 'queued': statusText = 'Queued'; break;
                case 'sent': statusText = 'Sent'; break;
                case 'delivered': statusText = 'Delivered'; break;
                case 'read': statusText = 'Read'; break;
                case 'failed': statusText = 'Failed'; break;
            }
            if (statusText) {
                bubble.appendChild(Ephemera.el('div', 'message-status', statusText));
            }
        }
        return bubble;
    }

    // ================================================================
    // Group Chat View
    // ================================================================

    async function renderGroupChatView(container) {
        container.innerHTML = '';

        var conv = currentConversation;
        if (!conv) { renderConversationList(container); return; }

        var chatView = Ephemera.el('div', 'chat-view');

        // Header
        var header = Ephemera.el('div', 'chat-header');
        var backBtn = Ephemera.el('button', 'chat-back');
        backBtn.innerHTML = '<svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2"><polyline points="15 18 9 12 15 6"/></svg>';
        backBtn.setAttribute('aria-label', 'Back to conversations');
        backBtn.addEventListener('click', function () {
            currentConversation = null;
            renderConversationList(container);
        });
        header.appendChild(backBtn);

        var displayName = conv.name || 'Group Chat';
        header.appendChild(Ephemera.avatar(displayName, 'avatar-sm'));

        var headerInfo = Ephemera.el('div', 'chat-header-info');
        headerInfo.appendChild(Ephemera.el('div', 'chat-header-name', displayName));
        var memberHint = Ephemera.el('div', 'chat-header-sub');
        memberHint.innerHTML = ICON_GROUP_CHAT;
        memberHint.appendChild(document.createTextNode(
            ' ' + (conv.member_count || conv.members_count || '?') + ' people'));
        headerInfo.appendChild(memberHint);
        header.appendChild(headerInfo);

        // Leave button
        var leaveBtn = Ephemera.el('button', 'btn btn-ghost btn-sm');
        leaveBtn.innerHTML = '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2"><path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"/><polyline points="16 17 21 12 16 7"/><line x1="21" y1="12" x2="9" y2="12"/></svg>';
        leaveBtn.title = 'Leave chat';
        leaveBtn.addEventListener('click', async function () {
            if (!confirm('Leave this group chat?')) return;
            try {
                await Ephemera.rpc('group_chats.leave', { chat_id: conv.chat_id || conv.id });
                Ephemera.showToast('Left group chat', 'info');
                currentConversation = null;
                renderConversationList(container);
            } catch (err) {
                Ephemera.showToast('Failed: ' + err.message, 'error');
            }
        });
        header.appendChild(leaveBtn);

        chatView.appendChild(header);

        // Messages
        var messagesArea = Ephemera.el('div', 'chat-messages');
        messagesArea.setAttribute('role', 'log');
        messagesArea.setAttribute('aria-label', 'Messages in ' + displayName);

        var chatId = conv.chat_id || conv.id || '';

        var loadingMsg = Ephemera.el('div', 'loading-state');
        loadingMsg.style.padding = '40px';
        loadingMsg.appendChild(Ephemera.el('div', 'spinner'));
        messagesArea.appendChild(loadingMsg);
        chatView.appendChild(messagesArea);

        loadGroupChatMessages(messagesArea, chatId, loadingMsg);

        // Compose
        var compose = Ephemera.el('div', 'chat-compose');
        var inputWrap = Ephemera.el('div', 'chat-input-wrap');
        var input = document.createElement('textarea');
        input.className = 'chat-input';
        input.placeholder = 'Message the group...';
        input.rows = 1;
        input.setAttribute('aria-label', 'Type a group message');
        inputWrap.appendChild(input);
        compose.appendChild(inputWrap);

        var sendBtn = Ephemera.el('button', 'chat-send-btn');
        sendBtn.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><line x1="22" y1="2" x2="11" y2="13"/><polygon points="22 2 15 22 11 13 2 9 22 2"/></svg>';
        sendBtn.setAttribute('aria-label', 'Send message');

        input.addEventListener('input', function () {
            input.style.height = 'auto';
            input.style.height = Math.min(input.scrollHeight, 120) + 'px';
        });

        async function doSend() {
            var body = input.value.trim();
            if (!body) return;
            sendBtn.disabled = true;
            try {
                await Ephemera.rpc('group_chats.send', { chat_id: chatId, body: body });
                var bubble = Ephemera.el('div', 'message-bubble sent');
                bubble.appendChild(document.createTextNode(body));
                bubble.appendChild(Ephemera.el('div', 'message-time', 'now'));
                messagesArea.appendChild(bubble);
                messagesArea.scrollTop = messagesArea.scrollHeight;
                input.value = '';
                input.style.height = 'auto';
            } catch (err) {
                Ephemera.showToast('Failed to send: ' + err.message, 'error');
            }
            sendBtn.disabled = false;
            input.focus();
        }

        sendBtn.addEventListener('click', doSend);
        input.addEventListener('keydown', function (e) {
            if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); doSend(); }
        });
        compose.appendChild(sendBtn);
        chatView.appendChild(compose);

        container.appendChild(chatView);
        input.focus();
    }

    async function loadGroupChatMessages(messagesArea, chatId, loadingEl) {
        try {
            var result = await Ephemera.rpc('group_chats.messages', {
                chat_id: chatId,
                limit: 50,
            });
            if (loadingEl && loadingEl.parentNode) loadingEl.remove();

            var messages = result.messages || result || [];

            if (!Array.isArray(messages) || messages.length === 0) {
                var empty = Ephemera.el('div', 'empty-state');
                empty.style.padding = '40px 20px';
                empty.appendChild(Ephemera.el('p', '', 'No messages yet. Start the conversation!'));
                messagesArea.appendChild(empty);
            } else {
                messages.forEach(function (msg) {
                    var bubble = Ephemera.el('div', 'message-bubble');
                    bubble.classList.add(msg.is_own ? 'sent' : 'received');

                    // Show sender name on received group messages
                    if (!msg.is_own) {
                        var senderName = msg.sender_display_name || msg.sender_handle
                            || (msg.sender ? msg.sender.slice(0, 8) + '...' : 'Someone');
                        bubble.appendChild(Ephemera.el('div', 'gc-sender-name', senderName));
                    }

                    bubble.appendChild(document.createTextNode(msg.body || ''));

                    if (msg.created_at) {
                        var msgMs = msg.created_at;
                        if (msgMs && msgMs < 1e12) msgMs = msgMs * 1000;
                        bubble.appendChild(Ephemera.el('div', 'message-time',
                            Ephemera.timeAgo(msgMs)));
                    }

                    messagesArea.appendChild(bubble);
                });
                messagesArea.scrollTop = messagesArea.scrollHeight;
            }
        } catch (_err) {
            if (loadingEl && loadingEl.parentNode) loadingEl.remove();
            var empty = Ephemera.el('div', 'empty-state');
            empty.style.padding = '40px 20px';
            empty.appendChild(Ephemera.el('p', '', 'Could not load messages.'));
            messagesArea.appendChild(empty);
        }
    }

    // ================================================================
    // Route
    // ================================================================

    Ephemera.registerRoute('/messages', function (container) {
        currentConversation = null;
        renderConversationList(container);
    });
})();
