// js/qr.js
let html5QrCode = null;
let isProcessingScan = false;

export async function startQRScan(mode, currentClassId, classes, onStudentScanned, showAlert) {
    isProcessingScan = false;
    document.getElementById('qr-modal-title').innerText = mode === 'asistencia' ? 'Pase de Lista QR' : 'Calificar por QR';
    document.getElementById('qr-modal-subtitle').innerText = 'Apunta la cámara al código QR';
    document.getElementById('qr-scan-status').classList.remove('hidden');
    document.getElementById('qr-grade-panel').classList.add('hidden');
    document.getElementById('reader').classList.remove('hidden');
    document.getElementById('qr-modal').classList.remove('hidden');

    try {
        if (navigator.mediaDevices && navigator.mediaDevices.getUserMedia) {
            const stream = await navigator.mediaDevices.getUserMedia({ video: true });
            stream.getTracks().forEach(t => t.stop());
        }

        if (html5QrCode) {
            try { if (html5QrCode.isScanning) await html5QrCode.stop(); } catch (_) {}
            try { await html5QrCode.clear(); } catch (_) {}
        }
        html5QrCode = new Html5Qrcode("reader");

        const qrConfig = { fps: 12, qrbox: { width: 210, height: 210 } };
        await html5QrCode.start({ facingMode: "environment" }, qrConfig, (text) => {
            if (isProcessingScan) return;
            isProcessingScan = true;
            onStudentScanned(text);
        }, () => {});

        document.getElementById('qr-scan-status').innerHTML = '<span class="text-green-500 font-bold">Cámara activa</span>';
    } catch (err) {
        document.getElementById('qr-scan-status').innerHTML = '<span class="text-red-500">No se pudo acceder a la cámara.</span>';
    }
}

export async function stopQRScan() {
    if (html5QrCode) {
        try {
            if (html5QrCode.isScanning) await html5QrCode.stop();
            html5QrCode.clear();
        } catch (_) {}
    }
    document.getElementById('qr-modal').classList.add('hidden');
    isProcessingScan = false;
}