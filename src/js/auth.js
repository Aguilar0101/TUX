// js/auth.js
import { invoke } from './api.js';

export async function loginProfesor(e, showAlert, onSuccess) {
    e.preventDefault();
    try {
        const user = await invoke('login_profesor', {
            email: document.getElementById('login-email').value,
            password: document.getElementById('login-pwd').value
        });
        onSuccess(user);
    } catch (e) {
        showAlert('Error', String(e), 'error');
    }
}

export async function registerProfesor(e, showAlert, onRegistered) {
    e.preventDefault();
    try {
        await invoke('registrar_profesor', {
            name: document.getElementById('reg-name').value,
            email: document.getElementById('reg-email').value,
            password: document.getElementById('reg-pwd').value,
            securityQuestion: document.getElementById('reg-question').value,
            securityAnswer: document.getElementById('reg-answer').value
        });
        showAlert('Éxito', 'Cuenta de profesor registrada.', 'success');
        onRegistered();
    } catch (e) {
        showAlert('Error', String(e), 'error');
    }
}

export async function buscarCuentaRecuperacion(e, showAlert) {
    e.preventDefault();
    const email = document.getElementById('fp-email').value.trim();
    const btn = document.getElementById('fp-send-btn');
    btn.disabled = true;
    btn.innerText = 'Enviando correo...';
    try {
        const info = await invoke('obtener_pregunta_seguridad', { email });
        document.getElementById('fp-question-label').innerText = info.question;
        document.getElementById('fp-step-1').classList.add('hidden');
        document.getElementById('fp-step-2').classList.remove('hidden');
        const extraMsg = info.debug_pin ? ` (Modo local sin SMTP: PIN ${info.debug_pin})` : ' Revisa tu correo.';
        showAlert('PIN Enviado', `Código enviado a ${email}.${extraMsg}`, 'info');
    } catch (error) {
        showAlert('Error de Envío', String(error), 'error');
    } finally {
        btn.disabled = false;
        btn.innerText = 'Enviar PIN por Correo';
    }
}

export async function confirmarRestablecerContrasena(e, showAlert, onSuccess) {
    e.preventDefault();
    const pin = document.getElementById('fp-pin').value.trim();
    const answer = document.getElementById('fp-answer').value.trim();
    const newPassword = document.getElementById('fp-new-pwd').value;
    if (newPassword !== document.getElementById('fp-confirm-pwd').value) {
        showAlert('Error', 'Las contraseñas no coinciden.', 'error');
        return;
    }
    try {
        await invoke('restablecer_contrasena', {
            email: document.getElementById('fp-email').value.trim(),
            answer,
            pin,
            newPassword
        });
        onSuccess();
        showAlert('Éxito', 'Contraseña restablecida correctamente.', 'success');
    } catch (error) {
        showAlert('Error', String(error), 'error');
    }
}