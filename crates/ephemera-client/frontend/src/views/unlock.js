/**
 * Ephemera -- Unlock View
 *
 * Shown when a keystore exists on disk but the identity is locked.
 * Prompts for the passphrase to unlock.
 *
 * RPC calls:
 *   - identity.unlock { passphrase }
 *   - identity.get_active {}
 */
(function () {
    'use strict';

    function renderUnlock(container) {
        container.innerHTML = '';

        var wrap = Ephemera.el('div', 'onboarding');

        wrap.appendChild(Ephemera.el('div', 'onboarding-logo', 'Ephemera'));
        wrap.appendChild(Ephemera.el('p', 'onboarding-tagline', 'Welcome back. Enter your passphrase to unlock.'));

        var stepDiv = Ephemera.el('div', 'onboarding-step');
        stepDiv.appendChild(Ephemera.el('h2', '', 'Unlock Identity'));
        stepDiv.appendChild(Ephemera.el('p', '',
            'Your identity is encrypted on this device. Enter your passphrase to continue.'));

        var group = Ephemera.el('div', 'input-group');
        var label = Ephemera.el('label', '', 'Passphrase');
        label.setAttribute('for', 'unlock-passphrase');
        group.appendChild(label);

        var input = document.createElement('input');
        input.type = 'password';
        input.id = 'unlock-passphrase';
        input.className = 'input-field';
        input.placeholder = 'Enter your passphrase';
        input.setAttribute('aria-describedby', 'unlock-hint');
        group.appendChild(input);

        var hint = Ephemera.el('div', '');
        hint.id = 'unlock-hint';
        hint.style.cssText = 'font-size:var(--fs-xs);color:var(--text-tertiary);margin-top:4px;';
        hint.textContent = 'The passphrase you chose when creating your identity.';
        group.appendChild(hint);

        stepDiv.appendChild(group);

        // "Remember me" checkbox
        var rememberGroup = Ephemera.el('div', '');
        rememberGroup.style.cssText = 'display:flex;align-items:center;gap:8px;margin-top:12px;';
        var rememberCheck = document.createElement('input');
        rememberCheck.type = 'checkbox';
        rememberCheck.id = 'remember-me';
        rememberCheck.checked = true; // default to checked for convenience
        rememberCheck.style.cssText = 'width:18px;height:18px;accent-color:var(--primary);';
        var rememberLabel = Ephemera.el('label', '', 'Remember me on this device');
        rememberLabel.setAttribute('for', 'remember-me');
        rememberLabel.style.cssText = 'font-size:var(--fs-sm);color:var(--text-secondary);cursor:pointer;';
        rememberGroup.appendChild(rememberCheck);
        rememberGroup.appendChild(rememberLabel);
        stepDiv.appendChild(rememberGroup);

        var errorMsg = Ephemera.el('div', '');
        errorMsg.style.cssText = 'font-size:var(--fs-sm);color:var(--error);margin-top:8px;min-height:20px;';
        stepDiv.appendChild(errorMsg);

        var unlockBtn = Ephemera.el('button', 'btn btn-primary btn-full', 'Unlock');
        unlockBtn.style.marginTop = '8px';

        async function doUnlock() {
            var passphrase = input.value;
            if (!passphrase) {
                errorMsg.textContent = 'Please enter your passphrase.';
                return;
            }

            unlockBtn.disabled = true;
            unlockBtn.textContent = 'Unlocking...';
            errorMsg.textContent = '';

            try {
                await Ephemera.rpc('identity.unlock', {
                    passphrase: passphrase,
                    remember: rememberCheck.checked,
                });

                // Fetch the full identity profile
                try {
                    var profile = await Ephemera.rpc('identity.get_active');
                    Ephemera.store.set({
                        identity: profile,
                        hasIdentity: true,
                        hasKeystore: true,
                    });
                } catch (_e) {
                    Ephemera.store.set({ hasIdentity: true, hasKeystore: true });
                }

                Ephemera.showToast('Welcome back!', 'success');
                Ephemera.navigate('/feed');
            } catch (err) {
                var msg = err.message || 'Unlock failed';
                if (msg.toLowerCase().indexOf('passphrase') !== -1 ||
                    msg.toLowerCase().indexOf('decrypt') !== -1 ||
                    msg.toLowerCase().indexOf('authentication') !== -1) {
                    errorMsg.textContent = 'Incorrect passphrase. Please try again.';
                } else {
                    errorMsg.textContent = 'Unlock failed: ' + msg;
                }
                unlockBtn.disabled = false;
                unlockBtn.textContent = 'Unlock';
                input.focus();
                input.select();
            }
        }

        unlockBtn.addEventListener('click', doUnlock);
        input.addEventListener('keydown', function (e) {
            if (e.key === 'Enter') {
                e.preventDefault();
                doUnlock();
            }
        });

        stepDiv.appendChild(unlockBtn);

        // Option to start fresh
        var spacer = Ephemera.el('div', '');
        spacer.style.height = '24px';
        stepDiv.appendChild(spacer);

        var newIdentityBtn = Ephemera.el('button', 'btn btn-ghost btn-full', 'Create New Identity Instead');
        newIdentityBtn.addEventListener('click', function () {
            Ephemera.store.set({ hasKeystore: false });
            Ephemera.navigate('/onboarding');
        });
        stepDiv.appendChild(newIdentityBtn);

        wrap.appendChild(stepDiv);
        container.appendChild(wrap);
        input.focus();
    }

    Ephemera.registerRoute('/unlock', renderUnlock);
})();
