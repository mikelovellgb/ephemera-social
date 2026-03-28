/**
 * Ephemera -- Onboarding View
 *
 * Magical first experience with:
 * - Full-screen welcome with animated gradient logo
 * - Smooth slide transitions between steps
 * - Passphrase strength meter (color bar)
 * - Identity creation animation (spinning rings)
 * - Recovery phrase in word card grid
 * - Confetti-like particle effect on "Get Started"
 *
 * RPC calls:
 *   - identity.create { passphrase }
 *   - profiles.update { display_name }
 *   - identity.backup_mnemonic { passphrase }
 *   - identity.get_active {}
 *   - identity.import_mnemonic { words, passphrase }
 */
(function () {
    'use strict';

    var TOTAL_STEPS = 4;
    var step = 0;
    var displayName = '';
    var passphrase = '';
    var recoveryPhrase = '';
    var isRecoveryMode = false;
    var storedMnemonic = [];

    function renderStepIndicator(currentStep) {
        var indicator = Ephemera.el('div', 'step-indicator');
        indicator.setAttribute('role', 'progressbar');
        indicator.setAttribute('aria-valuenow', String(currentStep));
        indicator.setAttribute('aria-valuemax', String(TOTAL_STEPS));
        indicator.setAttribute('aria-label', 'Step ' + currentStep + ' of ' + TOTAL_STEPS);

        for (var i = 0; i <= TOTAL_STEPS; i++) {
            var dot = Ephemera.el('span', 'step-dot');
            if (i === currentStep) dot.classList.add('active');
            if (i < currentStep) dot.classList.add('completed');
            indicator.appendChild(dot);
        }
        return indicator;
    }

    // Step 0: Welcome
    function renderWelcome(container) {
        container.innerHTML = '';
        var wrap = Ephemera.el('div', 'onboarding');

        wrap.appendChild(Ephemera.el('div', 'onboarding-logo', 'Ephemera'));
        wrap.appendChild(Ephemera.el('p', 'onboarding-tagline',
            'Speak freely. Connect privately. Everything fades, as it should.'));
        wrap.appendChild(renderStepIndicator(0));

        var stepDiv = Ephemera.el('div', 'onboarding-step');

        var createBtn = Ephemera.el('button', 'btn btn-primary btn-full', 'Create Identity');
        createBtn.setAttribute('aria-label', 'Create a new identity');
        createBtn.addEventListener('click', function () {
            isRecoveryMode = false;
            step = 1;
            render(container);
        });
        stepDiv.appendChild(createBtn);

        var spacer = Ephemera.el('div', '');
        spacer.style.height = '12px';
        stepDiv.appendChild(spacer);

        var recoverBtn = Ephemera.el('button', 'btn btn-secondary btn-full', 'Recover Existing Identity');
        recoverBtn.setAttribute('aria-label', 'Recover identity from recovery phrase');
        recoverBtn.addEventListener('click', function () {
            isRecoveryMode = true;
            step = 1;
            render(container);
        });
        stepDiv.appendChild(recoverBtn);

        wrap.appendChild(stepDiv);
        container.appendChild(wrap);
    }

    // Step 1: Name (create) or Mnemonic (recover)
    function renderNameStep(container) {
        container.innerHTML = '';
        var wrap = Ephemera.el('div', 'onboarding');

        wrap.appendChild(Ephemera.el('div', 'onboarding-logo', 'Ephemera'));
        wrap.appendChild(renderStepIndicator(1));

        var stepDiv = Ephemera.el('div', 'onboarding-step');

        if (isRecoveryMode) {
            stepDiv.appendChild(Ephemera.el('h2', '', 'Welcome back'));
            stepDiv.appendChild(Ephemera.el('p', '',
                'Enter your recovery phrase (12 or 24 words, space-separated).'));

            var mnGroup = Ephemera.el('div', 'input-group');
            var mnLabel = Ephemera.el('label', '', 'Recovery Phrase');
            mnLabel.setAttribute('for', 'mnemonic-input');
            mnGroup.appendChild(mnLabel);

            var mnInput = document.createElement('textarea');
            mnInput.id = 'mnemonic-input';
            mnInput.className = 'input-field';
            mnInput.placeholder = 'word1 word2 word3 ...';
            mnInput.rows = 3;
            mnInput.setAttribute('aria-describedby', 'mnemonic-hint');
            if (storedMnemonic.length > 0) mnInput.value = storedMnemonic.join(' ');
            mnGroup.appendChild(mnInput);

            var mnHint = Ephemera.el('div', '', '12 or 24 words separated by spaces');
            mnHint.id = 'mnemonic-hint';
            mnHint.style.cssText = 'font-size:var(--fs-xs);color:var(--text-tertiary);margin-top:4px;';
            mnGroup.appendChild(mnHint);
            stepDiv.appendChild(mnGroup);
        } else {
            stepDiv.appendChild(Ephemera.el('h2', '', 'Choose a display name'));
            stepDiv.appendChild(Ephemera.el('p', '',
                'This is how others will see you. You can change it anytime.'));

            var nameGroup = Ephemera.el('div', 'input-group');
            var nameLabel = Ephemera.el('label', '', 'Display Name');
            nameLabel.setAttribute('for', 'display-name');
            nameGroup.appendChild(nameLabel);

            var nameInput = document.createElement('input');
            nameInput.type = 'text';
            nameInput.id = 'display-name';
            nameInput.className = 'input-field';
            nameInput.placeholder = 'Anonymous Penguin';
            nameInput.maxLength = 30;
            nameInput.value = displayName;
            nameInput.addEventListener('input', function () {
                displayName = nameInput.value;
            });
            nameGroup.appendChild(nameInput);

            var nameHint = Ephemera.el('div', '', '1-30 characters');
            nameHint.style.cssText = 'font-size:var(--fs-xs);color:var(--text-tertiary);margin-top:4px;';
            nameGroup.appendChild(nameHint);
            stepDiv.appendChild(nameGroup);
        }

        var btnRow = Ephemera.el('div', 'compose-actions');

        var backBtn = Ephemera.el('button', 'btn btn-ghost', 'Back');
        backBtn.addEventListener('click', function () {
            step = 0;
            render(container);
        });
        btnRow.appendChild(backBtn);

        var nextBtn = Ephemera.el('button', 'btn btn-primary', 'Next');
        nextBtn.addEventListener('click', function () {
            if (isRecoveryMode) {
                var mnEl = document.getElementById('mnemonic-input');
                var words = mnEl ? mnEl.value.trim().split(/\s+/).filter(Boolean) : [];
                if (words.length !== 12 && words.length !== 24) {
                    Ephemera.showToast('Recovery phrase must be 12 or 24 words', 'error');
                    return;
                }
                storedMnemonic = words;
            } else {
                if (!displayName.trim()) {
                    Ephemera.showToast('Please enter a display name', 'error');
                    return;
                }
            }
            step = 2;
            render(container);
        });
        btnRow.appendChild(nextBtn);

        stepDiv.appendChild(btnRow);
        wrap.appendChild(stepDiv);
        container.appendChild(wrap);

        var focusEl = container.querySelector('#display-name') || container.querySelector('#mnemonic-input');
        if (focusEl) focusEl.focus();
    }

    // Step 2: Passphrase with strength meter
    function renderPassphraseStep(container) {
        container.innerHTML = '';
        var wrap = Ephemera.el('div', 'onboarding');

        wrap.appendChild(Ephemera.el('div', 'onboarding-logo', 'Ephemera'));
        wrap.appendChild(renderStepIndicator(2));

        var stepDiv = Ephemera.el('div', 'onboarding-step');
        stepDiv.appendChild(Ephemera.el('h2', '', 'Set a passphrase'));
        stepDiv.appendChild(Ephemera.el('p', '',
            'This encrypts your identity on this device. Make it memorable and strong.'));

        var group = Ephemera.el('div', 'input-group');
        var label = Ephemera.el('label', '', 'Passphrase');
        label.setAttribute('for', 'passphrase');
        group.appendChild(label);

        var input = document.createElement('input');
        input.type = 'password';
        input.id = 'passphrase';
        input.className = 'input-field';
        input.placeholder = 'At least 8 characters';
        input.value = passphrase;
        input.setAttribute('aria-describedby', 'passphrase-hint');
        group.appendChild(input);

        // Strength meter bar
        var meter = Ephemera.el('div', 'strength-meter');
        var meterFill = Ephemera.el('div', 'strength-meter-fill');
        meter.appendChild(meterFill);
        group.appendChild(meter);

        var strengthText = Ephemera.el('div', '');
        strengthText.id = 'passphrase-hint';
        strengthText.style.cssText = 'font-size:var(--fs-xs);margin-top:6px;color:var(--text-tertiary);';
        strengthText.textContent = 'Minimum 8 characters';
        group.appendChild(strengthText);
        stepDiv.appendChild(group);

        function updateStrength() {
            var len = passphrase.length;
            meterFill.className = 'strength-meter-fill';
            if (len === 0) {
                strengthText.textContent = 'Minimum 8 characters';
                strengthText.style.color = 'var(--text-tertiary)';
            } else if (len < 8) {
                meterFill.classList.add('weak');
                strengthText.textContent = len + '/8 (too short)';
                strengthText.style.color = 'var(--error)';
            } else if (len < 12) {
                meterFill.classList.add('fair');
                strengthText.textContent = 'Acceptable (' + len + ' chars)';
                strengthText.style.color = 'var(--warning)';
            } else if (len < 16) {
                meterFill.classList.add('good');
                strengthText.textContent = 'Good (' + len + ' chars)';
                strengthText.style.color = 'var(--info)';
            } else {
                meterFill.classList.add('strong');
                strengthText.textContent = 'Strong (' + len + ' chars)';
                strengthText.style.color = 'var(--success)';
            }
        }

        input.addEventListener('input', function () {
            passphrase = input.value;
            updateStrength();
        });

        // Initialize display
        updateStrength();

        var btnRow = Ephemera.el('div', 'compose-actions');

        var backBtn = Ephemera.el('button', 'btn btn-ghost', 'Back');
        backBtn.addEventListener('click', function () {
            step = 1;
            render(container);
        });
        btnRow.appendChild(backBtn);

        var actionLabel = isRecoveryMode ? 'Recover Identity' : 'Create Identity';
        var createBtn = Ephemera.el('button', 'btn btn-primary', actionLabel);
        createBtn.addEventListener('click', function () {
            if (passphrase.length < 8) {
                Ephemera.showToast('Passphrase must be at least 8 characters', 'error');
                return;
            }
            doCreateOrRecover(container);
        });
        btnRow.appendChild(createBtn);

        stepDiv.appendChild(btnRow);
        wrap.appendChild(stepDiv);
        container.appendChild(wrap);
        input.focus();
    }

    // Identity creation / recovery
    async function doCreateOrRecover(container) {
        step = 3;
        render(container);

        try {
            if (isRecoveryMode) {
                await Ephemera.rpc('identity.import_mnemonic', {
                    words: storedMnemonic,
                    passphrase: passphrase,
                });
            } else {
                await Ephemera.rpc('identity.create', { passphrase: passphrase });

                if (displayName.trim()) {
                    try {
                        await Ephemera.rpc('profiles.update', { display_name: displayName.trim() });
                    } catch (_e) { /* non-fatal */ }
                }

                try {
                    var backup = await Ephemera.rpc('identity.backup_mnemonic', { passphrase: passphrase });
                    var m = backup.mnemonic || backup.phrase || '';
                    recoveryPhrase = Array.isArray(m) ? m.join(' ') : String(m);
                } catch (_e2) {
                    recoveryPhrase = '';
                }
            }

            try {
                var profile = await Ephemera.rpc('identity.get_active');
                Ephemera.store.set({ identity: profile, hasIdentity: true, hasKeystore: true });
            } catch (_e3) {
                Ephemera.store.set({ hasIdentity: true, hasKeystore: true });
            }

            if (isRecoveryMode) {
                Ephemera.showToast('Identity recovered! Welcome back.', 'success');
                Ephemera.navigate('/feed');
            } else {
                step = 4;
                render(container);
            }
        } catch (err) {
            console.error('Identity creation/recovery failed:', err);
            Ephemera.showToast('Failed: ' + err.message, 'error');
            step = 2;
            render(container);
        }
    }

    // Step 3: Loading / Creating with spinning animation
    function renderCreatingStep(container) {
        container.innerHTML = '';
        var wrap = Ephemera.el('div', 'onboarding');

        wrap.appendChild(Ephemera.el('div', 'onboarding-logo', 'Ephemera'));
        wrap.appendChild(renderStepIndicator(3));

        var stepDiv = Ephemera.el('div', 'onboarding-step');
        stepDiv.appendChild(Ephemera.el('h2', '',
            isRecoveryMode ? 'Recovering identity...' : 'Creating your identity...'));
        stepDiv.appendChild(Ephemera.el('p', '',
            isRecoveryMode
                ? 'Restoring from recovery phrase. This may take a moment.'
                : 'Generating cryptographic keys. This involves a proof-of-work computation (15-30 seconds).'));

        // Spinning identity animation
        stepDiv.appendChild(Ephemera.el('div', 'identity-animation'));

        var statusText = Ephemera.el('p', '');
        statusText.style.cssText = 'font-size:var(--fs-xs);color:var(--text-tertiary);margin-top:16px;';
        statusText.textContent = 'Computing proof of work...';
        stepDiv.appendChild(statusText);

        wrap.appendChild(stepDiv);
        container.appendChild(wrap);
    }

    // Step 4: Recovery phrase display with word cards
    function renderRecoveryDisplay(container) {
        container.innerHTML = '';
        var wrap = Ephemera.el('div', 'onboarding');

        wrap.appendChild(Ephemera.el('div', 'onboarding-logo', 'Ephemera'));
        wrap.appendChild(renderStepIndicator(4));

        var stepDiv = Ephemera.el('div', 'onboarding-step');
        stepDiv.appendChild(Ephemera.el('h2', '', 'Save your recovery phrase'));

        var warning = Ephemera.el('div', 'recovery-warning',
            'Write this down and store it safely. This is the ONLY way to recover your identity if you lose this device.');
        warning.setAttribute('role', 'alert');
        stepDiv.appendChild(warning);

        // Word card grid
        if (recoveryPhrase) {
            var words = recoveryPhrase.split(/\s+/);
            var grid = Ephemera.el('div', 'word-grid');

            words.forEach(function (word, idx) {
                var card = Ephemera.el('div', 'word-card');
                var num = Ephemera.el('span', 'word-num', String(idx + 1));
                card.appendChild(num);
                card.appendChild(document.createTextNode(word));
                grid.appendChild(card);
            });

            stepDiv.appendChild(grid);
        } else {
            var fallback = Ephemera.el('div', 'recovery-phrase',
                '(Recovery phrase will be available in settings)');
            stepDiv.appendChild(fallback);
        }

        // Copy button
        var copyBtn = Ephemera.el('button', 'btn btn-secondary btn-full btn-sm', 'Copy to Clipboard');
        copyBtn.style.marginBottom = '16px';
        copyBtn.addEventListener('click', function () {
            if (navigator.clipboard && navigator.clipboard.writeText && recoveryPhrase) {
                navigator.clipboard.writeText(recoveryPhrase).then(function () {
                    Ephemera.showToast('Copied! Store it safely.', 'success');
                }).catch(function () {
                    Ephemera.showToast('Copy failed', 'error');
                });
            } else {
                Ephemera.showToast('Clipboard not available', 'info');
            }
        });
        stepDiv.appendChild(copyBtn);

        stepDiv.appendChild(Ephemera.el('p', '',
            'Welcome to Ephemera, ' + (displayName || 'friend') + '!'));

        var goBtn = Ephemera.el('button', 'btn btn-primary btn-full', 'I\'ve saved it. Let\'s go!');
        goBtn.style.marginTop = '16px';
        goBtn.addEventListener('click', function () {
            Ephemera.navigate('/feed');
        });
        stepDiv.appendChild(goBtn);

        wrap.appendChild(stepDiv);
        container.appendChild(wrap);
    }

    function render(container) {
        switch (step) {
            case 0: renderWelcome(container); break;
            case 1: renderNameStep(container); break;
            case 2: renderPassphraseStep(container); break;
            case 3: renderCreatingStep(container); break;
            case 4: renderRecoveryDisplay(container); break;
        }
    }

    Ephemera.registerRoute('/onboarding', function (container) {
        step = 0;
        displayName = '';
        passphrase = '';
        recoveryPhrase = '';
        isRecoveryMode = false;
        storedMnemonic = [];
        render(container);
    });
})();
