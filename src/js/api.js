// js/api.js
export const IS_TAURI = !!window.__TAURI__;

export const invoke = window.__TAURI__?.core?.invoke || (async (cmd, args = {}) => {
    const simUsers = JSON.parse(localStorage.getItem('sim_sqlite_users') || '{}');
    const simClasses = JSON.parse(localStorage.getItem('sim_sqlite_classes') || '[]');
    const simSmtp = JSON.parse(localStorage.getItem('sim_smtp_config') || '{"host":"smtp.gmail.com","user":"","pass":""}');

    if (cmd === 'guardar_config_smtp') {
        localStorage.setItem('sim_smtp_config', JSON.stringify({ host: args.host, user: args.user, pass: args.pass }));
        return null;
    }
    if (cmd === 'obtener_config_smtp') return simSmtp;
    if (cmd === 'registrar_profesor') {
        if (simUsers[args.email]) throw "El correo ya está registrado.";
        simUsers[args.email] = { name: args.name, password: args.password, security_question: (args.securityQuestion || '').trim(), security_answer: (args.securityAnswer || '').trim().toLowerCase() };
        localStorage.setItem('sim_sqlite_users', JSON.stringify(simUsers));
        return null;
    }
    if (cmd === 'login_profesor') {
        const u = simUsers[args.email];
        if (!u || u.password !== args.password) throw "Credenciales incorrectas";
        return { email: args.email, name: u.name, role: 'profesor' };
    }
    if (cmd === 'obtener_pregunta_seguridad') {
        const u = simUsers[args.email];
        if (!u) throw "No existe cuenta con ese correo.";
        const pin = Math.floor(100000 + Math.random() * 900000).toString();
        u.reset_pin = pin;
        localStorage.setItem('sim_sqlite_users', JSON.stringify(simUsers));
        return { email: args.email, question: u.security_question || "¿Nombre completo?", has_custom_question: Boolean(u.security_question), debug_pin: pin };
    }
    if (cmd === 'restablecer_contrasena') {
        const u = simUsers[args.email];
        if (!u) throw "Cuenta no encontrada.";
        if (u.reset_pin && u.reset_pin !== args.pin) throw "El PIN de verificación es incorrecto.";
        u.password = args.newPassword;
        delete u.reset_pin;
        localStorage.setItem('sim_sqlite_users', JSON.stringify(simUsers));
        return null;
    }
    if (cmd === 'obtener_clases_profesor') return simClasses.filter(c => c.teacher === args.teacherEmail);
    if (cmd === 'importar_alumnos_csv') {
        const cls = simClasses.find(c => c.id === args.clase.id);
        if (!cls) return 0;
        if (!cls.students) cls.students = [];
        let count = 0;
        args.alumnos.forEach(a => {
            if (!cls.students.some(s => s.account === a.account)) { cls.students.push({ account: a.account, name: a.name }); count++; }
        });
        localStorage.setItem('sim_sqlite_classes', JSON.stringify(simClasses));
        return count;
    }
    return null;
});

export function localDateStr(d = new Date()) {
    const p = n => String(n).padStart(2, '0');
    return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

export async function saveBinaryFile(filename, arrayBuffer, mime = 'application/octet-stream') {
    if (IS_TAURI) {
        const bytes = Array.from(new Uint8Array(arrayBuffer));
        return await invoke('guardar_archivo', { filename, bytes });
    }
    const url = URL.createObjectURL(new Blob([arrayBuffer], { type: mime }));
    const a = document.createElement('a');
    a.href = url; a.download = filename;
    document.body.appendChild(a); a.click(); a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 2000);
    return filename;
}