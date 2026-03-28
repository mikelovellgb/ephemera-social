/**
 * Ephemera -- QR Code Scanner
 *
 * Provides QR code scanning using:
 * 1. The native BarcodeDetector API (Chrome 83+, Android WebView, Safari 15.4+)
 * 2. Manual entry fallback for environments without camera or BarcodeDetector
 *
 * The decoded payload is expected to be either:
 * - A hex string (74 chars = 37 bytes) for identity.import_qr
 * - An invite link: ephemera://connect/<pubkey_hex>
 *
 * On Android, the system camera app can also handle ephemera:// deep links
 * directly, which is the most reliable path for QR-to-connection.
 */
(function () {
    'use strict';

    /**
     * Check if the BarcodeDetector API supports QR codes.
     * @returns {Promise<boolean>}
     */
    async function hasBarcodeDetector() {
        if (typeof BarcodeDetector === 'undefined') return false;
        try {
            var formats = await BarcodeDetector.getSupportedFormats();
            return formats.indexOf('qr_code') !== -1;
        } catch (_e) {
            return false;
        }
    }

    /**
     * Open a camera modal, scan for QR codes, and return the decoded text.
     *
     * Uses the native BarcodeDetector API when available. Falls back to
     * showing only the manual entry option when BarcodeDetector is not
     * supported.
     *
     * @returns {Promise<string|null>} The decoded QR text, or null if cancelled.
     */
    function openScanner() {
        return new Promise(function (resolve) {
            var overlay = Ephemera.el('div', 'modal-overlay qr-scanner-overlay');
            overlay.setAttribute('role', 'dialog');
            overlay.setAttribute('aria-modal', 'true');
            overlay.setAttribute('aria-label', 'QR Code Scanner');

            var modal = Ephemera.el('div', 'modal-content qr-scanner-modal');
            modal.appendChild(Ephemera.el('h2', '', 'Scan QR Code'));

            var instructions = Ephemera.el('p', 'qr-instructions',
                'Point your camera at the QR code from the other device.');
            instructions.style.cssText = 'font-size:0.85rem;color:var(--text-secondary);margin-bottom:12px;';
            modal.appendChild(instructions);

            // Video preview
            var videoWrap = Ephemera.el('div', 'qr-video-wrap');
            videoWrap.style.cssText = 'position:relative;width:100%;max-width:320px;' +
                'margin:0 auto;border-radius:var(--radius-md);overflow:hidden;' +
                'background:#000;aspect-ratio:1/1;';

            var video = document.createElement('video');
            video.setAttribute('autoplay', '');
            video.setAttribute('playsinline', '');
            video.setAttribute('muted', '');
            video.style.cssText = 'width:100%;height:100%;object-fit:cover;';
            videoWrap.appendChild(video);

            // Viewfinder overlay
            var viewfinder = Ephemera.el('div', 'qr-viewfinder');
            viewfinder.style.cssText = 'position:absolute;top:15%;left:15%;width:70%;height:70%;' +
                'border:2px solid var(--accent);border-radius:var(--radius-md);' +
                'pointer-events:none;box-shadow:0 0 0 9999px rgba(0,0,0,0.4);';
            videoWrap.appendChild(viewfinder);

            modal.appendChild(videoWrap);

            // Status text
            var status = Ephemera.el('p', 'qr-status', 'Requesting camera access...');
            status.style.cssText = 'text-align:center;font-size:0.85rem;margin-top:12px;' +
                'color:var(--text-secondary);';
            modal.appendChild(status);

            // Manual entry fallback (always visible, prominent)
            var fallbackHint = Ephemera.el('p', '');
            fallbackHint.style.cssText = 'text-align:center;font-size:0.8rem;margin-top:8px;' +
                'color:var(--text-tertiary);';
            fallbackHint.textContent = 'Camera not working? Enter the code or invite link manually.';
            modal.appendChild(fallbackHint);

            // Actions
            var actionsRow = Ephemera.el('div', 'modal-actions');

            var cancelBtn = Ephemera.el('button', 'btn btn-ghost', 'Cancel');
            cancelBtn.addEventListener('click', function () { cleanup(null); });
            actionsRow.appendChild(cancelBtn);

            // Manual entry fallback button
            var manualBtn = Ephemera.el('button', 'btn btn-secondary', 'Enter Manually');
            manualBtn.addEventListener('click', function () {
                cleanup(null);
                openManualEntry().then(resolve);
            });
            actionsRow.appendChild(manualBtn);
            modal.appendChild(actionsRow);

            overlay.appendChild(modal);

            // Close on background click
            overlay.addEventListener('click', function (e) {
                if (e.target === overlay) cleanup(null);
            });

            // Close on Escape
            function onEsc(e) {
                if (e.key === 'Escape') cleanup(null);
            }
            document.addEventListener('keydown', onEsc);

            document.body.appendChild(overlay);

            var stream = null;
            var scanInterval = null;
            var closed = false;
            var detector = null;

            function cleanup(result) {
                if (closed) return;
                closed = true;
                if (scanInterval) clearInterval(scanInterval);
                if (stream) {
                    stream.getTracks().forEach(function (t) { t.stop(); });
                }
                document.removeEventListener('keydown', onEsc);
                overlay.remove();
                resolve(result);
            }

            // Start camera + BarcodeDetector
            startCameraScanning();

            async function startCameraScanning() {
                // Check BarcodeDetector support first
                var canDetect = await hasBarcodeDetector();

                if (!canDetect) {
                    // No BarcodeDetector -- hide video, show prominent manual entry
                    videoWrap.style.display = 'none';
                    status.textContent = 'QR scanning is not supported on this device.';
                    status.style.color = 'var(--text-error, #f44)';
                    fallbackHint.textContent = 'Use "Enter Manually" to paste the invite link or code.';
                    instructions.textContent = 'Tip: On Android, use your phone\'s camera app to scan QR codes -- it will open Ephemera automatically via the deep link.';
                    return;
                }

                detector = new BarcodeDetector({ formats: ['qr_code'] });

                // Check camera access
                if (!navigator.mediaDevices || !navigator.mediaDevices.getUserMedia) {
                    videoWrap.style.display = 'none';
                    status.textContent = 'Camera not available on this device.';
                    status.style.color = 'var(--text-error, #f44)';
                    return;
                }

                try {
                    var mediaStream = await navigator.mediaDevices.getUserMedia({
                        video: { facingMode: 'environment', width: { ideal: 640 }, height: { ideal: 640 } },
                    });

                    if (closed) {
                        mediaStream.getTracks().forEach(function (t) { t.stop(); });
                        return;
                    }

                    stream = mediaStream;
                    video.srcObject = stream;
                    status.textContent = 'Scanning...';

                    // Scan frames at ~4 fps using BarcodeDetector
                    scanInterval = setInterval(async function () {
                        if (closed) return;
                        if (video.readyState < video.HAVE_ENOUGH_DATA) return;

                        try {
                            var barcodes = await detector.detect(video);
                            if (barcodes.length > 0) {
                                var rawValue = barcodes[0].rawValue;
                                if (rawValue) {
                                    status.textContent = 'QR code found!';
                                    status.style.color = 'var(--accent)';
                                    cleanup(rawValue);
                                }
                            }
                        } catch (_detectErr) {
                            // Detection can fail on some frames -- ignore
                        }
                    }, 250);

                } catch (err) {
                    console.error('Camera access error:', err);
                    videoWrap.style.display = 'none';
                    status.textContent = 'Camera access denied.';
                    status.style.color = 'var(--text-error, #f44)';
                    fallbackHint.textContent = 'Use "Enter Manually" to paste the invite link or code.';
                }
            }
        });
    }

    /**
     * Manual entry modal. Accepts both:
     * - Raw hex (74 chars for identity import)
     * - Invite links (ephemera://connect/<pubkey>)
     * - Plain pubkey hex (64 chars)
     *
     * @returns {Promise<string|null>}
     */
    function openManualEntry() {
        return new Promise(function (resolve) {
            var overlay = Ephemera.el('div', 'modal-overlay');
            overlay.setAttribute('role', 'dialog');
            overlay.setAttribute('aria-modal', 'true');
            overlay.setAttribute('aria-label', 'Enter code manually');

            var modal = Ephemera.el('div', 'modal-content');
            modal.appendChild(Ephemera.el('h2', '', 'Enter Code Manually'));
            modal.appendChild(Ephemera.el('p', '',
                'Paste the invite link or hex code from the other device.'));

            var group = Ephemera.el('div', 'input-group');
            var label = Ephemera.el('label', '', 'Invite Link or Hex Code');
            label.setAttribute('for', 'qr-manual-input');
            group.appendChild(label);

            var input = document.createElement('input');
            input.type = 'text';
            input.id = 'qr-manual-input';
            input.className = 'input-field';
            input.placeholder = 'ephemera://connect/... or hex code';
            input.setAttribute('aria-label', 'Invite link or hex code');
            input.style.fontFamily = 'var(--font-mono)';
            group.appendChild(input);
            modal.appendChild(group);

            var actionsRow = Ephemera.el('div', 'modal-actions');
            var cancelBtn = Ephemera.el('button', 'btn btn-ghost', 'Cancel');
            cancelBtn.addEventListener('click', function () {
                overlay.remove();
                resolve(null);
            });
            actionsRow.appendChild(cancelBtn);

            var submitBtn = Ephemera.el('button', 'btn btn-primary', 'Submit');
            submitBtn.addEventListener('click', function () {
                var val = input.value.trim();
                if (!val) {
                    Ephemera.showToast('Enter a code or invite link', 'error');
                    return;
                }
                overlay.remove();
                resolve(val);
            });
            actionsRow.appendChild(submitBtn);

            // Handle Enter key
            input.addEventListener('keydown', function (e) {
                if (e.key === 'Enter') submitBtn.click();
            });

            modal.appendChild(actionsRow);
            overlay.appendChild(modal);

            overlay.addEventListener('click', function (e) {
                if (e.target === overlay) {
                    overlay.remove();
                    resolve(null);
                }
            });

            function onEsc(e) {
                if (e.key === 'Escape') {
                    overlay.remove();
                    document.removeEventListener('keydown', onEsc);
                    resolve(null);
                }
            }
            document.addEventListener('keydown', onEsc);

            document.body.appendChild(overlay);
            input.focus();
        });
    }

    /**
     * Full scan + import flow for identity QR codes.
     *
     * Opens the scanner, gets the result, and dispatches based on content:
     * - If it looks like an invite link or pubkey: establish network + social connection
     * - If it looks like a raw hex blob (74 chars): call identity.import_qr
     *
     * @param {string} passphrase The passphrase for the new local keystore (for import).
     * @returns {Promise<{success: boolean, pubkey?: string, error?: string}>}
     */
    async function scanAndImport(passphrase) {
        var scanned = await openScanner();
        if (!scanned) {
            return { success: false, error: 'cancelled' };
        }

        // Determine what was scanned
        var trimmed = scanned.trim();

        // Case 1: Invite link -- extract pubkey and connect
        if (trimmed.startsWith('ephemera://connect/')) {
            var pubkey = trimmed.replace('ephemera://connect/', '').split('?')[0].trim();
            return await connectScannedPubkey(pubkey);
        }

        // Case 2: 64-char hex string -- likely a pubkey (connect)
        if (/^[0-9a-fA-F]{64}$/.test(trimmed)) {
            return await connectScannedPubkey(trimmed);
        }

        // Case 3: 74-char hex string -- identity import QR
        if (/^[0-9a-fA-F]{74}$/.test(trimmed)) {
            try {
                var result = await Ephemera.rpc('identity.import_qr', {
                    qr_hex: trimmed,
                    passphrase: passphrase,
                });

                if (result && result.imported) {
                    return {
                        success: true,
                        pubkey: result.pseudonym_pubkey,
                    };
                }

                return { success: false, error: 'unexpected response' };
            } catch (err) {
                return { success: false, error: err.message || 'import failed' };
            }
        }

        // Case 4: Unknown format
        return { success: false, error: 'Unrecognized QR code format: ' + trimmed.slice(0, 40) };
    }

    /**
     * Establish network + social connection with a scanned pubkey.
     * @param {string} pubkey Hex-encoded public key
     * @returns {Promise<{success: boolean, pubkey?: string, error?: string}>}
     */
    async function connectScannedPubkey(pubkey) {
        try {
            // Step 1: Establish network connection via Iroh discovery
            try {
                await Ephemera.rpc('network.connect', { node_id: pubkey });
            } catch (e) {
                console.warn('QR scan: network connect failed (peer may be offline):', e.message || e);
            }

            // Step 2: Send social connection request
            await Ephemera.rpc('social.connect', {
                target: pubkey,
                message: 'Connected via QR code!',
            });

            Ephemera.showToast('Connection request sent!', 'success');
            Ephemera.navigate('/discover');
            return { success: true, pubkey: pubkey };
        } catch (err) {
            Ephemera.showToast('Connection failed: ' + err.message, 'error');
            return { success: false, error: err.message || 'connect failed' };
        }
    }

    // Expose on global Ephemera object
    Ephemera.QRScanner = {
        openScanner: openScanner,
        openManualEntry: openManualEntry,
        scanAndImport: scanAndImport,
    };
})();
