/**
 * Ephemera -- Groups View
 *
 * Full groups experience:
 * - My Groups list with avatars, handles, member counts
 * - Browse/Search public groups
 * - Create Group modal (name, description, visibility)
 * - Group detail view: header, feed, members, admin controls
 * - Post to group from within the group
 * - Admin: set roles, kick, ban members
 *
 * RPC calls:
 *   - groups.list {}
 *   - groups.create { name, description, visibility }
 *   - groups.join { group_id }
 *   - groups.leave { group_id }
 *   - groups.search { query }
 *   - groups.info { group_id }
 *   - groups.feed { group_id, limit }
 *   - groups.post { group_id, content_hash }
 *   - groups.set_role { group_id, target, role }
 *   - groups.kick { group_id, target }
 *   - groups.ban { group_id, target, reason }
 *   - groups.invite { group_id, target }
 *   - groups.delete { group_id }
 *   - groups.register_handle { group_id, handle }
 *   - posts.create { body, ttl_seconds, audience }
 */
(function () {
    'use strict';

    // ================================================================
    // Group Card — used in lists
    // ================================================================

    function groupAvatar(name) {
        var div = Ephemera.el('div', 'avatar avatar-md group-avatar-icon');
        div.setAttribute('aria-hidden', 'true');
        var initials = (name || '?').split(/\s+/).map(function (w) { return w[0]; }).join('').toUpperCase().slice(0, 2);
        div.innerHTML = '<svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.8" style="opacity:0.6;"><path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M23 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/></svg>';
        div.title = initials;
        return div;
    }

    function renderGroupCard(group, parentContainer) {
        var card = Ephemera.el('div', 'group-card');
        card.setAttribute('role', 'button');
        card.setAttribute('tabindex', '0');
        card.setAttribute('aria-label', group.name + ' group');

        card.appendChild(groupAvatar(group.name));

        var info = Ephemera.el('div', 'group-card-info');

        var nameRow = Ephemera.el('div', 'group-card-name-row');
        nameRow.appendChild(Ephemera.el('span', 'group-card-name', group.name));
        if (group.visibility === 'private' || group.visibility === 'secret') {
            var lockIcon = Ephemera.el('span', 'group-lock-icon');
            lockIcon.innerHTML = '<svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.5"><rect x="3" y="11" width="18" height="11" rx="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/></svg>';
            lockIcon.title = group.visibility;
            nameRow.appendChild(lockIcon);
        }
        info.appendChild(nameRow);

        if (group.handle) {
            info.appendChild(Ephemera.el('div', 'group-card-handle', '@' + group.handle));
        }
        if (group.description) {
            var desc = group.description.length > 80
                ? group.description.slice(0, 80) + '...'
                : group.description;
            info.appendChild(Ephemera.el('div', 'group-card-desc', desc));
        }

        var meta = Ephemera.el('div', 'group-card-meta');
        var memberCount = Ephemera.el('span', '');
        memberCount.innerHTML = '<svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2"><path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/></svg> ' + (group.member_count || 0);
        meta.appendChild(memberCount);
        if (group.my_role) {
            var roleBadge = Ephemera.el('span', 'role-badge role-' + group.my_role, group.my_role);
            meta.appendChild(roleBadge);
        }
        info.appendChild(meta);

        card.appendChild(info);

        // Join/Leave button on the right
        var actionArea = Ephemera.el('div', 'group-card-action');
        if (!group.is_member && !group.my_role) {
            var joinBtn = Ephemera.el('button', 'btn btn-primary btn-sm', 'Join');
            joinBtn.addEventListener('click', function (e) {
                e.stopPropagation();
                joinBtn.disabled = true;
                joinBtn.textContent = 'Joining...';
                Ephemera.rpc('groups.join', { group_id: group.group_id }).then(function () {
                    Ephemera.showToast('Joined ' + group.name + '!', 'success');
                    joinBtn.textContent = 'Joined';
                    joinBtn.className = 'btn btn-ghost btn-sm';
                    joinBtn.disabled = true;
                }).catch(function (err) {
                    Ephemera.showToast('Failed: ' + err.message, 'error');
                    joinBtn.disabled = false;
                    joinBtn.textContent = 'Join';
                });
            });
            actionArea.appendChild(joinBtn);
        }
        card.appendChild(actionArea);

        function openGroup() {
            renderGroupDetail(parentContainer, group.group_id);
        }
        card.addEventListener('click', openGroup);
        card.addEventListener('keydown', function (e) {
            if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); openGroup(); }
        });

        return card;
    }

    // ================================================================
    // Create Group Modal
    // ================================================================

    function showCreateGroupModal(onCreated) {
        var overlay = Ephemera.el('div', 'modal-overlay');
        overlay.setAttribute('role', 'dialog');
        overlay.setAttribute('aria-modal', 'true');
        overlay.setAttribute('aria-label', 'Create a group');

        var modal = Ephemera.el('div', 'modal-content');
        modal.appendChild(Ephemera.el('h2', '', 'Create Group'));
        modal.appendChild(Ephemera.el('p', '', 'Start a community around a shared interest.'));

        // Name
        var nameGroup = Ephemera.el('div', 'input-group');
        nameGroup.appendChild(Object.assign(Ephemera.el('label', '', 'Group Name'), { htmlFor: 'cg-name' }));
        var nameInput = document.createElement('input');
        nameInput.type = 'text';
        nameInput.id = 'cg-name';
        nameInput.className = 'input-field';
        nameInput.placeholder = 'Photography Club';
        nameInput.maxLength = 50;
        nameGroup.appendChild(nameInput);
        modal.appendChild(nameGroup);

        // Description
        var descGroup = Ephemera.el('div', 'input-group');
        descGroup.appendChild(Object.assign(Ephemera.el('label', '', 'Description'), { htmlFor: 'cg-desc' }));
        var descInput = document.createElement('textarea');
        descInput.id = 'cg-desc';
        descInput.className = 'input-field';
        descInput.placeholder = 'What is this group about?';
        descInput.maxLength = 500;
        descInput.rows = 3;
        descInput.style.minHeight = '80px';
        descGroup.appendChild(descInput);
        modal.appendChild(descGroup);

        // Visibility
        var visGroup = Ephemera.el('div', 'input-group');
        visGroup.appendChild(Ephemera.el('label', '', 'Visibility'));
        var visRow = Ephemera.el('div', 'visibility-selector');
        var visOptions = [
            { key: 'public', label: 'Public', desc: 'Anyone can find and join' },
            { key: 'private', label: 'Private', desc: 'Visible, but invite-only' },
            { key: 'secret', label: 'Secret', desc: 'Hidden, invite-only' },
        ];
        var selectedVis = 'public';
        visOptions.forEach(function (opt) {
            var btn = Ephemera.el('button', 'vis-option' + (opt.key === selectedVis ? ' active' : ''));
            btn.innerHTML = '<strong>' + opt.label + '</strong><span>' + opt.desc + '</span>';
            btn.addEventListener('click', function () {
                selectedVis = opt.key;
                visRow.querySelectorAll('.vis-option').forEach(function (b) { b.classList.remove('active'); });
                btn.classList.add('active');
            });
            visRow.appendChild(btn);
        });
        visGroup.appendChild(visRow);
        modal.appendChild(visGroup);

        // Actions
        var actionsRow = Ephemera.el('div', 'modal-actions');
        var cancelBtn = Ephemera.el('button', 'btn btn-ghost', 'Cancel');
        cancelBtn.addEventListener('click', function () { overlay.remove(); });
        actionsRow.appendChild(cancelBtn);

        var createBtn = Ephemera.el('button', 'btn btn-primary', 'Create');
        createBtn.addEventListener('click', async function () {
            var name = nameInput.value.trim();
            if (!name) {
                Ephemera.showToast('Group name is required', 'error');
                return;
            }
            createBtn.disabled = true;
            createBtn.textContent = 'Creating...';
            try {
                var result = await Ephemera.rpc('groups.create', {
                    name: name,
                    description: descInput.value.trim() || undefined,
                    visibility: selectedVis,
                });
                Ephemera.showToast('Group created!', 'success');
                overlay.remove();
                if (onCreated) onCreated(result);
            } catch (err) {
                Ephemera.showToast('Failed: ' + err.message, 'error');
                createBtn.disabled = false;
                createBtn.textContent = 'Create';
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
    // Group Detail View
    // ================================================================

    async function renderGroupDetail(container, groupId) {
        container.innerHTML = '';

        // Back header
        var header = Ephemera.el('div', 'group-detail-nav');
        var backBtn = Ephemera.el('button', 'chat-back');
        backBtn.innerHTML = '<svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2"><polyline points="15 18 9 12 15 6"/></svg>';
        backBtn.setAttribute('aria-label', 'Back to groups');
        backBtn.addEventListener('click', function () { renderGroups(container); });
        header.appendChild(backBtn);
        header.appendChild(Ephemera.el('span', 'group-detail-nav-title', 'Group'));
        container.appendChild(header);

        var loading = Ephemera.el('div', 'loading-state');
        loading.appendChild(Ephemera.el('div', 'spinner'));
        loading.appendChild(Ephemera.el('p', '', 'Loading group...'));
        container.appendChild(loading);

        try {
            var info = await Ephemera.rpc('groups.info', { group_id: groupId });
            loading.remove();

            // Group header card
            var headerCard = Ephemera.el('div', 'group-detail-header');
            var bigAvatar = groupAvatar(info.name);
            bigAvatar.className = 'avatar avatar-xl group-avatar-icon';
            headerCard.appendChild(bigAvatar);

            var headerInfo = Ephemera.el('div', 'group-detail-header-info');
            headerInfo.appendChild(Ephemera.el('h1', 'group-detail-name', info.name));
            if (info.handle) {
                headerInfo.appendChild(Ephemera.el('div', 'group-detail-handle', '@' + info.handle));
            }
            if (info.description) {
                headerInfo.appendChild(Ephemera.el('div', 'group-detail-desc', info.description));
            }

            var statRow = Ephemera.el('div', 'group-detail-stats');
            statRow.appendChild(Ephemera.el('span', 'group-stat', (info.member_count || 0) + ' members'));
            var visBadge = Ephemera.el('span', 'group-stat group-vis-badge');
            visBadge.textContent = info.visibility || 'public';
            statRow.appendChild(visBadge);
            if (info.my_role) {
                statRow.appendChild(Ephemera.el('span', 'role-badge role-' + info.my_role, info.my_role));
            }
            headerInfo.appendChild(statRow);
            headerCard.appendChild(headerInfo);
            container.appendChild(headerCard);

            // Action buttons
            var actions = Ephemera.el('div', 'group-detail-actions');
            var isAdmin = info.my_role === 'admin' || info.my_role === 'owner';

            if (!info.my_role) {
                var joinBtn = Ephemera.el('button', 'btn btn-primary', 'Join Group');
                joinBtn.addEventListener('click', async function () {
                    joinBtn.disabled = true;
                    try {
                        await Ephemera.rpc('groups.join', { group_id: groupId });
                        Ephemera.showToast('Joined!', 'success');
                        renderGroupDetail(container, groupId);
                    } catch (err) {
                        Ephemera.showToast('Failed: ' + err.message, 'error');
                        joinBtn.disabled = false;
                    }
                });
                actions.appendChild(joinBtn);
            } else {
                var leaveBtn = Ephemera.el('button', 'btn btn-ghost btn-sm', 'Leave');
                leaveBtn.addEventListener('click', async function () {
                    if (!confirm('Leave ' + info.name + '?')) return;
                    leaveBtn.disabled = true;
                    try {
                        await Ephemera.rpc('groups.leave', { group_id: groupId });
                        Ephemera.showToast('Left group', 'info');
                        renderGroups(container);
                    } catch (err) {
                        Ephemera.showToast('Failed: ' + err.message, 'error');
                        leaveBtn.disabled = false;
                    }
                });
                actions.appendChild(leaveBtn);

                var inviteBtn = Ephemera.el('button', 'btn btn-secondary btn-sm', 'Invite');
                inviteBtn.addEventListener('click', function () {
                    showInviteModal(groupId, info.name);
                });
                actions.appendChild(inviteBtn);

                if (isAdmin) {
                    var settingsBtn = Ephemera.el('button', 'btn btn-ghost btn-sm');
                    settingsBtn.innerHTML = '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg> Settings';
                    settingsBtn.addEventListener('click', function () {
                        showAdminPanel(container, groupId, info);
                    });
                    actions.appendChild(settingsBtn);
                }
            }
            container.appendChild(actions);

            // Tab bar: Feed | Members
            var tabBar = Ephemera.el('div', 'group-tab-bar');
            var feedTab = Ephemera.el('button', 'group-tab active', 'Feed');
            var membersTab = Ephemera.el('button', 'group-tab', 'Members (' + (info.member_count || 0) + ')');
            tabBar.appendChild(feedTab);
            tabBar.appendChild(membersTab);
            container.appendChild(tabBar);

            var tabContent = Ephemera.el('div', 'group-tab-content');
            container.appendChild(tabContent);

            feedTab.addEventListener('click', function () {
                feedTab.classList.add('active');
                membersTab.classList.remove('active');
                renderGroupFeed(tabContent, groupId, info.my_role);
            });
            membersTab.addEventListener('click', function () {
                membersTab.classList.add('active');
                feedTab.classList.remove('active');
                renderMembersList(tabContent, info, groupId, isAdmin);
            });

            // Default to feed
            renderGroupFeed(tabContent, groupId, info.my_role);

        } catch (err) {
            loading.remove();
            var errEl = Ephemera.el('div', 'empty-state');
            errEl.appendChild(Ephemera.el('h3', '', 'Could not load group'));
            errEl.appendChild(Ephemera.el('p', '', err.message || 'Unknown error'));
            container.appendChild(errEl);
        }
    }

    // ================================================================
    // Group Feed Tab
    // ================================================================

    async function renderGroupFeed(container, groupId, myRole) {
        container.innerHTML = '';

        // Compose inline (if member)
        if (myRole) {
            var composeRow = Ephemera.el('div', 'group-compose-row');
            var composeInput = document.createElement('input');
            composeInput.type = 'text';
            composeInput.className = 'input-field';
            composeInput.placeholder = 'Write something to the group...';
            composeInput.maxLength = 2000;
            composeRow.appendChild(composeInput);

            var sendBtn = Ephemera.el('button', 'btn btn-primary btn-sm', 'Post');
            sendBtn.addEventListener('click', async function () {
                var body = composeInput.value.trim();
                if (!body) return;
                sendBtn.disabled = true;
                sendBtn.textContent = 'Posting...';
                try {
                    var post = await Ephemera.rpc('posts.create', {
                        body: body,
                        ttl_seconds: 2592000,
                        audience: 'group:' + groupId,
                    });
                    var contentHash = post.content_hash || (post.result && post.result.content_hash) || '';
                    if (contentHash) {
                        await Ephemera.rpc('groups.post', {
                            group_id: groupId,
                            content_hash: contentHash,
                        });
                    }
                    Ephemera.showToast('Posted to group!', 'success');
                    composeInput.value = '';
                    renderGroupFeed(container, groupId, myRole);
                } catch (err) {
                    Ephemera.showToast('Failed: ' + err.message, 'error');
                }
                sendBtn.disabled = false;
                sendBtn.textContent = 'Post';
            });
            composeRow.appendChild(sendBtn);
            container.appendChild(composeRow);
        }

        var feedList = Ephemera.el('div', 'group-feed-list');
        feedList.appendChild(Ephemera.el('div', 'spinner'));
        container.appendChild(feedList);

        try {
            var feed = await Ephemera.rpc('groups.feed', { group_id: groupId, limit: 30 });
            feedList.innerHTML = '';

            var items = feed.items || feed.posts || [];
            if (items.length === 0) {
                var empty = Ephemera.el('div', 'empty-state');
                empty.style.padding = '40px 20px';
                var icon = Ephemera.el('div', 'empty-state-icon');
                icon.innerHTML = '<svg viewBox="0 0 24 24" width="48" height="48" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>';
                empty.appendChild(icon);
                empty.appendChild(Ephemera.el('h3', '', 'No posts yet'));
                empty.appendChild(Ephemera.el('p', '', myRole ? 'Be the first to post in this group!' : 'Join to see and create posts.'));
                feedList.appendChild(empty);
            } else {
                items.forEach(function (item) {
                    var postCard = Ephemera.el('div', 'group-feed-post');
                    var authorLabel = item.author_handle
                        ? '@' + item.author_handle
                        : (item.author ? item.author.slice(0, 8) + '...' : 'anon');

                    var postHeader = Ephemera.el('div', 'group-feed-post-header');
                    postHeader.appendChild(Ephemera.avatar(item.author_display_name || 'A', 'avatar-sm'));
                    var postInfo = Ephemera.el('div', '');
                    postInfo.appendChild(Ephemera.el('span', 'group-feed-post-author', item.author_display_name || authorLabel));
                    if (item.created_at) {
                        var ts = item.created_at;
                        if (ts < 1e12) ts = ts * 1000;
                        postInfo.appendChild(Ephemera.el('span', 'group-feed-post-time', ' ' + Ephemera.timeAgo(ts)));
                    }
                    postHeader.appendChild(postInfo);
                    postCard.appendChild(postHeader);

                    if (item.body) {
                        var bodyEl = Ephemera.el('div', 'group-feed-post-body');
                        bodyEl.innerHTML = Ephemera.highlightMentions(item.body);
                        postCard.appendChild(bodyEl);
                    }

                    feedList.appendChild(postCard);
                });
            }
        } catch (err) {
            feedList.innerHTML = '';
            feedList.appendChild(Ephemera.el('div', 'empty-state',
                'Could not load feed: ' + err.message));
        }
    }

    // ================================================================
    // Members List Tab
    // ================================================================

    function renderMembersList(container, info, groupId, isAdmin) {
        container.innerHTML = '';

        var members = info.members || [];
        if (members.length === 0) {
            container.appendChild(Ephemera.el('div', 'empty-state', 'No members found'));
            return;
        }

        var list = Ephemera.el('div', 'group-members-list');
        members.forEach(function (m) {
            var row = Ephemera.el('div', 'group-member-row');

            var displayName = m.display_name || m.handle || (m.pubkey ? m.pubkey.slice(0, 8) + '...' + m.pubkey.slice(-4) : 'Unknown');
            row.appendChild(Ephemera.avatar(displayName, 'avatar-sm'));

            var memberInfo = Ephemera.el('div', 'group-member-info');
            memberInfo.appendChild(Ephemera.el('div', 'group-member-name', displayName));
            if (m.handle) {
                memberInfo.appendChild(Ephemera.el('div', 'group-member-handle', '@' + m.handle));
            }
            row.appendChild(memberInfo);

            var roleBadge = Ephemera.el('span', 'role-badge role-' + (m.role || 'member'), m.role || 'member');
            row.appendChild(roleBadge);

            // Admin actions
            if (isAdmin && m.role !== 'owner') {
                var moreBtn = Ephemera.el('button', 'btn btn-ghost btn-sm member-action-btn');
                moreBtn.innerHTML = '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="5" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="12" cy="19" r="1"/></svg>';
                moreBtn.addEventListener('click', function (e) {
                    e.stopPropagation();
                    showMemberActionMenu(row, m, groupId, container, info, isAdmin);
                });
                row.appendChild(moreBtn);
            }

            list.appendChild(row);
        });
        container.appendChild(list);
    }

    // ================================================================
    // Member action menu (admin)
    // ================================================================

    function showMemberActionMenu(anchorEl, member, groupId, tabContent, info, isAdmin) {
        // Remove any existing popover
        var existing = document.querySelector('.member-popover');
        if (existing) existing.remove();

        var popover = Ephemera.el('div', 'member-popover');

        var promoteBtn = Ephemera.el('button', 'member-popover-item', 'Promote to Admin');
        promoteBtn.addEventListener('click', async function () {
            popover.remove();
            try {
                await Ephemera.rpc('groups.set_role', { group_id: groupId, target: member.pubkey, role: 'admin' });
                Ephemera.showToast('Promoted!', 'success');
                // Refresh info
                var refreshed = await Ephemera.rpc('groups.info', { group_id: groupId });
                renderMembersList(tabContent, refreshed, groupId, isAdmin);
            } catch (err) {
                Ephemera.showToast('Failed: ' + err.message, 'error');
            }
        });
        popover.appendChild(promoteBtn);

        if (member.role === 'admin') {
            var demoteBtn = Ephemera.el('button', 'member-popover-item', 'Demote to Member');
            demoteBtn.addEventListener('click', async function () {
                popover.remove();
                try {
                    await Ephemera.rpc('groups.set_role', { group_id: groupId, target: member.pubkey, role: 'member' });
                    Ephemera.showToast('Demoted', 'info');
                    var refreshed = await Ephemera.rpc('groups.info', { group_id: groupId });
                    renderMembersList(tabContent, refreshed, groupId, isAdmin);
                } catch (err) {
                    Ephemera.showToast('Failed: ' + err.message, 'error');
                }
            });
            popover.appendChild(demoteBtn);
        }

        var kickBtn = Ephemera.el('button', 'member-popover-item popover-danger', 'Kick');
        kickBtn.addEventListener('click', async function () {
            popover.remove();
            if (!confirm('Kick this member?')) return;
            try {
                await Ephemera.rpc('groups.kick', { group_id: groupId, target: member.pubkey });
                Ephemera.showToast('Kicked', 'info');
                var refreshed = await Ephemera.rpc('groups.info', { group_id: groupId });
                renderMembersList(tabContent, refreshed, groupId, isAdmin);
            } catch (err) {
                Ephemera.showToast('Failed: ' + err.message, 'error');
            }
        });
        popover.appendChild(kickBtn);

        var banBtn = Ephemera.el('button', 'member-popover-item popover-danger', 'Ban');
        banBtn.addEventListener('click', async function () {
            popover.remove();
            var reason = prompt('Ban reason (optional):');
            try {
                await Ephemera.rpc('groups.ban', {
                    group_id: groupId,
                    target: member.pubkey,
                    reason: reason || undefined,
                });
                Ephemera.showToast('Banned', 'info');
                var refreshed = await Ephemera.rpc('groups.info', { group_id: groupId });
                renderMembersList(tabContent, refreshed, groupId, isAdmin);
            } catch (err) {
                Ephemera.showToast('Failed: ' + err.message, 'error');
            }
        });
        popover.appendChild(banBtn);

        anchorEl.style.position = 'relative';
        anchorEl.appendChild(popover);

        // Close on click outside
        function closePopover(e) {
            if (!popover.contains(e.target)) {
                popover.remove();
                document.removeEventListener('click', closePopover, true);
            }
        }
        setTimeout(function () {
            document.addEventListener('click', closePopover, true);
        }, 0);
    }

    // ================================================================
    // Invite Modal
    // ================================================================

    function showInviteModal(groupId, groupName) {
        var overlay = Ephemera.el('div', 'modal-overlay');
        overlay.setAttribute('role', 'dialog');
        overlay.setAttribute('aria-modal', 'true');

        var modal = Ephemera.el('div', 'modal-content');
        modal.appendChild(Ephemera.el('h2', '', 'Invite to ' + groupName));
        modal.appendChild(Ephemera.el('p', '', 'Enter a pseudonym ID or @handle to invite.'));

        var input = document.createElement('input');
        input.type = 'text';
        input.className = 'input-field';
        input.placeholder = 'Pseudonym ID or @handle...';
        modal.appendChild(input);

        var actionsRow = Ephemera.el('div', 'modal-actions');
        var cancelBtn = Ephemera.el('button', 'btn btn-ghost', 'Cancel');
        cancelBtn.addEventListener('click', function () { overlay.remove(); });
        actionsRow.appendChild(cancelBtn);

        var sendBtn = Ephemera.el('button', 'btn btn-primary', 'Invite');
        sendBtn.addEventListener('click', async function () {
            var target = input.value.trim();
            if (!target) { Ephemera.showToast('Enter a target', 'error'); return; }

            // Resolve handle if needed
            if (target.startsWith('@') || target.startsWith('#')) {
                try {
                    var handleName = target.replace(/^[@#]/, '').toLowerCase().trim();
                    var lookup = await Ephemera.rpc('identity.lookup_handle', { name: handleName });
                    target = lookup.owner || lookup.owner_pubkey || lookup.pubkey || target;
                } catch (_e) {
                    // Use as-is
                }
            }

            sendBtn.disabled = true;
            try {
                await Ephemera.rpc('groups.invite', { group_id: groupId, target: target });
                Ephemera.showToast('Invitation sent!', 'success');
                overlay.remove();
            } catch (err) {
                Ephemera.showToast('Failed: ' + err.message, 'error');
                sendBtn.disabled = false;
            }
        });
        actionsRow.appendChild(sendBtn);
        modal.appendChild(actionsRow);

        overlay.appendChild(modal);
        overlay.addEventListener('click', function (e) { if (e.target === overlay) overlay.remove(); });
        document.body.appendChild(overlay);
        input.focus();
    }

    // ================================================================
    // Admin Settings Panel
    // ================================================================

    function showAdminPanel(pageContainer, groupId, info) {
        var overlay = Ephemera.el('div', 'modal-overlay');
        overlay.setAttribute('role', 'dialog');
        overlay.setAttribute('aria-modal', 'true');

        var modal = Ephemera.el('div', 'modal-content');
        modal.appendChild(Ephemera.el('h2', '', 'Group Settings'));

        // Register handle
        var handleGroup = Ephemera.el('div', 'input-group');
        handleGroup.appendChild(Ephemera.el('label', '', 'Group Handle'));
        var handleRow = Ephemera.el('div', '');
        handleRow.style.cssText = 'display:flex;gap:8px;';
        var handleInput = document.createElement('input');
        handleInput.type = 'text';
        handleInput.className = 'input-field';
        handleInput.placeholder = info.handle ? info.handle : 'e.g. photography-club';
        handleInput.maxLength = 30;
        handleRow.appendChild(handleInput);
        var registerBtn = Ephemera.el('button', 'btn btn-secondary btn-sm', 'Register');
        registerBtn.addEventListener('click', async function () {
            var h = handleInput.value.trim().toLowerCase().replace(/[^a-z0-9_-]/g, '');
            if (!h) { Ephemera.showToast('Enter a handle', 'error'); return; }
            registerBtn.disabled = true;
            try {
                await Ephemera.rpc('groups.register_handle', { group_id: groupId, handle: h });
                Ephemera.showToast('Handle registered!', 'success');
                handleInput.placeholder = h;
                handleInput.value = '';
            } catch (err) {
                Ephemera.showToast('Failed: ' + err.message, 'error');
            }
            registerBtn.disabled = false;
        });
        handleRow.appendChild(registerBtn);
        handleGroup.appendChild(handleRow);
        modal.appendChild(handleGroup);

        // Delete group (danger zone)
        var dangerZone = Ephemera.el('div', 'danger-zone');
        dangerZone.appendChild(Ephemera.el('h3', '', 'Danger Zone'));
        var deleteBtn = Ephemera.el('button', 'btn btn-danger', 'Delete Group');
        deleteBtn.addEventListener('click', async function () {
            if (!confirm('Are you sure? This cannot be undone.')) return;
            deleteBtn.disabled = true;
            try {
                await Ephemera.rpc('groups.delete', { group_id: groupId });
                Ephemera.showToast('Group deleted', 'info');
                overlay.remove();
                renderGroups(pageContainer);
            } catch (err) {
                Ephemera.showToast('Failed: ' + err.message, 'error');
                deleteBtn.disabled = false;
            }
        });
        dangerZone.appendChild(deleteBtn);
        modal.appendChild(dangerZone);

        var closeBtn = Ephemera.el('button', 'btn btn-ghost btn-full', 'Close');
        closeBtn.style.marginTop = '12px';
        closeBtn.addEventListener('click', function () { overlay.remove(); });
        modal.appendChild(closeBtn);

        overlay.appendChild(modal);
        overlay.addEventListener('click', function (e) { if (e.target === overlay) overlay.remove(); });
        function onEsc(e) {
            if (e.key === 'Escape') { overlay.remove(); document.removeEventListener('keydown', onEsc); }
        }
        document.addEventListener('keydown', onEsc);
        document.body.appendChild(overlay);
    }

    // ================================================================
    // Main Groups List View
    // ================================================================

    async function renderGroups(container) {
        container.innerHTML = '';

        // Header
        var header = Ephemera.el('div', 'discover-header');
        header.appendChild(Ephemera.el('h1', '', 'Groups'));

        var createBtn = Ephemera.el('button', 'btn btn-primary btn-sm');
        createBtn.innerHTML = '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2.5"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg> Create';
        createBtn.addEventListener('click', function () {
            showCreateGroupModal(function () { renderGroups(container); });
        });
        header.appendChild(createBtn);
        container.appendChild(header);

        // Search bar
        var searchBar = Ephemera.el('div', 'search-bar');
        searchBar.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/></svg>';
        var searchInput = document.createElement('input');
        searchInput.type = 'text';
        searchInput.className = 'input-field';
        searchInput.placeholder = 'Search public groups...';
        searchInput.setAttribute('aria-label', 'Search groups');
        searchBar.appendChild(searchInput);
        container.appendChild(searchBar);

        // Search handler
        var searchTimeout = null;
        searchInput.addEventListener('input', function () {
            clearTimeout(searchTimeout);
            var query = searchInput.value.trim();
            if (query.length < 2) {
                var existingResults = container.querySelector('.group-search-results');
                if (existingResults) existingResults.remove();
                return;
            }
            searchTimeout = setTimeout(async function () {
                try {
                    var result = await Ephemera.rpc('groups.search', { query: query });
                    var groups = result.groups || [];
                    var existingResults = container.querySelector('.group-search-results');
                    if (existingResults) existingResults.remove();

                    if (groups.length > 0) {
                        var resultsSection = Ephemera.el('div', 'group-search-results');
                        resultsSection.appendChild(Ephemera.el('div', 'section-title', 'Search Results'));
                        groups.forEach(function (g) {
                            resultsSection.appendChild(renderGroupCard(g, container));
                        });
                        // Insert after search bar
                        searchBar.parentNode.insertBefore(resultsSection, searchBar.nextSibling);
                    }
                } catch (_err) {
                    // silent
                }
            }, 400);
        });

        // My Groups section
        container.appendChild(Ephemera.el('div', 'section-title', 'My Groups'));
        var myList = Ephemera.el('div', 'groups-list');
        var myLoading = Ephemera.el('div', 'loading-state');
        myLoading.appendChild(Ephemera.el('div', 'spinner'));
        myList.appendChild(myLoading);
        container.appendChild(myList);

        try {
            var result = await Ephemera.rpc('groups.list');
            myList.innerHTML = '';
            var groups = result.groups || [];
            if (groups.length === 0) {
                var empty = Ephemera.el('div', 'empty-state');
                empty.style.padding = '32px 16px';
                var icon = Ephemera.el('div', 'empty-state-icon');
                icon.innerHTML = '<svg viewBox="0 0 24 24" width="48" height="48" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M23 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/></svg>';
                empty.appendChild(icon);
                empty.appendChild(Ephemera.el('h3', '', 'No groups yet'));
                empty.appendChild(Ephemera.el('p', '', 'Create a group or search for public ones to join.'));
                myList.appendChild(empty);
            } else {
                groups.forEach(function (g) {
                    myList.appendChild(renderGroupCard(g, container));
                });
            }
        } catch (err) {
            myList.innerHTML = '';
            myList.appendChild(Ephemera.el('div', 'empty-state', 'Could not load groups'));
        }
    }

    Ephemera.registerRoute('/groups', renderGroups);
})();
