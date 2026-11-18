// src-tauri/src/server.rs
//
// Servidor HTTP local para que los alumnos entren al portal desde su celular
// conectándose al hotspot (o a la misma red Wi-Fi) del profesor.

use std::collections::HashMap;
use std::io::Read;
use std::net::UdpSocket;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use rand::Rng;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{command, State};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

const PORT: u16 = 8080;
const PORTAL_HTML: &str = include_str!("portal.html");
const MAX_FALLOS: u32 = 10;
const BLOQUEO_SEG: u64 = 60;

// ───────────────────────── Estado compartido ─────────────────────────

#[derive(Default)]
struct Shared {
    teacher: String,
    classes: Vec<Value>,
    sessions: HashMap<String, String>,       // token -> cuenta
    fallos: HashMap<String, (u32, Instant)>, // ip -> (intentos fallidos, último intento)
}

type SharedRef = Arc<Mutex<Shared>>;

fn lock(m: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Default)]
pub struct ServerState {
    data: SharedRef,
    server: Mutex<Option<Arc<Server>>>,
}

#[derive(Serialize)]
pub struct ServerInfo {
    running: bool,
    ip: Option<String>,
    port: u16,
    url: Option<String>,
}

fn aplicar_datos(sh: &mut Shared, datos: &str) -> Result<(), String> {
    let v: Value = serde_json::from_str(datos).map_err(|e| format!("Datos inválidos: {}", e))?;
    sh.teacher = v
        .get("teacher")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    sh.classes = v
        .get("classes")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(())
}

// ───────────────────────── Detección de IP ─────────────────────────

fn ip_hacia(destino: &str) -> Option<String> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect(destino).ok()?;
    let ip = sock.local_addr().ok()?.ip();
    if ip.is_loopback() || ip.is_unspecified() {
        None
    } else {
        Some(ip.to_string())
    }
}

fn ip_local() -> Option<String> {
    for h in ["192.168.43.1", "192.168.137.1", "172.20.10.1", "192.168.49.1"] {
        if let Some(ip) = ip_hacia(&format!("{}:80", h)) {
            if ip == h {
                return Some(ip);
            }
        }
    }
    for destino in ["8.8.8.8:80", "192.168.1.1:80", "192.168.0.1:80", "10.0.0.1:80"] {
        if let Some(ip) = ip_hacia(destino) {
            return Some(ip);
        }
    }
    None
}

fn construir_info(running: bool) -> ServerInfo {
    let ip = if running { ip_local() } else { None };
    let url = ip.as_ref().map(|i| format!("http://{}:{}", i, PORT));
    ServerInfo { running, ip, port: PORT, url }
}

// ───────────────────────── Comandos Tauri ─────────────────────────

#[command]
pub fn servidor_alumnos_iniciar(
    state: State<'_, ServerState>,
    datos: String,
) -> Result<ServerInfo, String> {
    {
        let mut sh = lock(&state.data);
        aplicar_datos(&mut sh, &datos)?;
    }
    let mut run = state.server.lock().unwrap_or_else(|e| e.into_inner());
    if run.is_none() {
        let server = Server::http(("0.0.0.0", PORT)).map_err(|e| {
            format!(
                "No se pudo abrir el puerto {} (¿ya está en uso o el firewall lo bloquea?): {}",
                PORT, e
            )
        })?;
        let server = Arc::new(server);
        let s2 = Arc::clone(&server);
        let d2 = Arc::clone(&state.data);
        std::thread::spawn(move || bucle(s2, d2));
        *run = Some(server);
    }
    Ok(construir_info(true))
}

#[command]
pub fn servidor_alumnos_actualizar(
    state: State<'_, ServerState>,
    datos: String,
) -> Result<(), String> {
    let mut sh = lock(&state.data);
    aplicar_datos(&mut sh, &datos)
}

#[command]
pub fn servidor_alumnos_detener(state: State<'_, ServerState>) -> Result<(), String> {
    let mut run = state.server.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = run.take() {
        s.unblock();
    }
    lock(&state.data).sessions.clear();
    Ok(())
}

#[command]
pub fn servidor_alumnos_estado(state: State<'_, ServerState>) -> ServerInfo {
    let running = state
        .server
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some();
    construir_info(running)
}

// ───────────────────────── Bucle HTTP ─────────────────────────

fn bucle(server: Arc<Server>, data: SharedRef) {
    for mut req in server.incoming_requests() {
        let metodo = req.method().clone();
        let url = req.url().to_string();
        let ip = req
            .remote_addr()
            .map(|a| a.ip().to_string())
            .unwrap_or_default();
        let data = Arc::clone(&data);
        std::thread::spawn(move || {
            let mut cuerpo = String::new();
            if metodo == Method::Post {
                let _ = req.as_reader().take(65536).read_to_string(&mut cuerpo);
            }
            atender(req, metodo, url, cuerpo, ip, data);
        });
    }
}

fn header(k: &str, v: &str) -> Header {
    Header::from_bytes(k.as_bytes(), v.as_bytes()).unwrap()
}

fn responder(
    req: Request,
    status: u16,
    ctype: &str,
    body: Vec<u8>,
    extra: Option<(&str, String)>,
) {
    let mut r = Response::from_data(body)
        .with_status_code(StatusCode(status))
        .with_header(header("Content-Type", ctype))
        .with_header(header("Cache-Control", "no-store"));
    if let Some((k, v)) = extra {
        r = r.with_header(header(k, &v));
    }
    let _ = req.respond(r);
}

fn responder_json(req: Request, status: u16, v: Value) {
    responder(
        req,
        status,
        "application/json; charset=utf-8",
        v.to_string().into_bytes(),
        None,
    );
}

fn atender(req: Request, metodo: Method, url: String, cuerpo: String, ip: String, data: SharedRef) {
    let (ruta, query) = match url.split_once('?') {
        Some((r, q)) => (r.to_string(), q.to_string()),
        None => (url.clone(), String::new()),
    };
    match (metodo, ruta.as_str()) {
        (Method::Get, "/") => responder(
            req,
            200,
            "text/html; charset=utf-8",
            PORTAL_HTML.as_bytes().to_vec(),
            None,
        ),
        (Method::Post, "/api/login") => api_login(req, &cuerpo, &ip, &data),
        (Method::Get, "/reporte.pdf") => api_reporte(req, &query, &data),
        (Method::Get, "/favicon.ico") => responder(req, 204, "image/x-icon", Vec::new(), None),
        _ => responder(
            req,
            404,
            "text/plain; charset=utf-8",
            b"No encontrado".to_vec(),
            None,
        ),
    }
}

// ───────────────────────── API ─────────────────────────

fn api_login(req: Request, cuerpo: &str, ip: &str, data: &SharedRef) {
    let cuenta = serde_json::from_str::<Value>(cuerpo)
        .ok()
        .and_then(|v| v.get("account").map(val_str))
        .unwrap_or_default()
        .trim()
        .to_string();

    let mut sh = lock(data);

    let bloqueado = match sh.fallos.get(ip) {
        Some((n, t)) => *n >= MAX_FALLOS && t.elapsed() < Duration::from_secs(BLOQUEO_SEG),
        None => false,
    };
    if bloqueado {
        drop(sh);
        return responder_json(
            req,
            429,
            json!({ "error": "Demasiados intentos. Espera un minuto e intenta de nuevo." }),
        );
    }

    match buscar_nombre(&sh.classes, &cuenta) {
        None => {
            let e = sh
                .fallos
                .entry(ip.to_string())
                .or_insert((0, Instant::now()));
            if e.0 >= MAX_FALLOS && e.1.elapsed() >= Duration::from_secs(BLOQUEO_SEG) {
                e.0 = 0;
            }
            e.0 += 1;
            e.1 = Instant::now();
            drop(sh);
            responder_json(req, 401, json!({ "error": "Número de cuenta no encontrado." }))
        }
        Some(nombre) => {
            sh.fallos.remove(ip);
            if sh.sessions.len() > 5000 {
                sh.sessions.clear();
            }
            let token = nuevo_token();
            sh.sessions.insert(token.clone(), cuenta.clone());
            let clases = resumen_clases(&sh.classes, &cuenta, None);
            drop(sh);
            responder_json(
                req,
                200,
                json!({ "token": token, "name": nombre, "account": cuenta, "classes": clases }),
            )
        }
    }
}

fn api_reporte(req: Request, query: &str, data: &SharedRef) {
    let params = parsear_query(query);
    let token = params.get("token").cloned().unwrap_or_default();
    let clase = params.get("class").cloned();
    let fecha = params
        .get("fecha")
        .cloned()
        .filter(|f| f.len() == 10 && f.chars().all(|c| c.is_ascii_digit() || c == '-'))
        .unwrap_or_default();

    let sh = lock(data);
    let cuenta = sh.sessions.get(&token).cloned();
    let cuenta = match cuenta {
        Some(c) => c,
        None => {
            drop(sh);
            return responder(
                req,
                401,
                "text/plain; charset=utf-8",
                "Sesión expirada. Vuelve a ingresar.".as_bytes().to_vec(),
                None,
            );
        }
    };
    let nombre = buscar_nombre(&sh.classes, &cuenta).unwrap_or_default();
    let clases = resumen_clases(&sh.classes, &cuenta, clase.as_deref());
    let profesor = sh.teacher.clone();
    drop(sh);

    if clases.is_empty() {
        return responder(
            req,
            404,
            "text/plain; charset=utf-8",
            b"No hay datos para ese reporte".to_vec(),
            None,
        );
    }

    let pdf = generar_pdf(&bloques_reporte(&nombre, &cuenta, &profesor, &fecha, &clases));
    let archivo: String = format!("Reporte_{}.pdf", cuenta)
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '_' { c } else { '_' })
        .collect();
    responder(
        req,
        200,
        "application/pdf",
        pdf,
        Some((
            "Content-Disposition",
            format!("attachment; filename=\"{}\"", archivo),
        )),
    );
}

// ───────────────────────── Utilidades ─────────────────────────

fn nuevo_token() -> String {
    format!("{:032x}", rand::thread_rng().gen::<u128>())
}

fn val_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn decodificar(s: &str) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                match u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or(""), 16) {
                    Ok(v) => { out.push(v); i += 3; }
                    Err(_) => { out.push(b'%'); i += 1; }
                }
            }
            b'+' => { out.push(b' '); i += 1; }
            c => { out.push(c); i += 1; }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

fn parsear_query(q: &str) -> HashMap<String, String> {
    q.split('&')
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (decodificar(k), decodificar(v)))
        .collect()
}

fn buscar_nombre(clases: &[Value], cuenta: &str) -> Option<String> {
    if cuenta.is_empty() { return None; }
    for c in clases {
        if let Some(alumnos) = c.get("students").and_then(|s| s.as_array()) {
            for a in alumnos {
                if a.get("account").map(val_str).unwrap_or_default().trim() == cuenta {
                    return Some(a.get("name").map(val_str).unwrap_or_default());
                }
            }
        }
    }
    None
}

fn nota_num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => {
            let t = s.trim();
            if t == "entregado" { Some(10.0) } else { t.parse::<f64>().ok() }
        }
        _ => None,
    }
}

fn estado_actividad(v: Option<&Value>) -> (String, String) {
    match v {
        None | Some(Value::Null) => ("falta".into(), "No entregado (0)".into()),
        Some(Value::Number(n)) => ("nota".into(), n.to_string()),
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() || t == "no_entregado" {
                ("falta".into(), "No entregado (0)".into())
            } else if t == "entregado" {
                ("entregado".into(), "Entregado (10)".into())
            } else {
                ("nota".into(), t.to_string())
            }
        }
        Some(o) => ("nota".into(), o.to_string()),
    }
}

fn partes_fecha(f: &str) -> Option<(&'static str, u32, u32, i32)> {
    let p: Vec<&str> = f.split('-').collect();
    if p.len() != 3 { return None; }
    let y = p[0].parse::<i32>().ok()?;
    let m = p[1].parse::<u32>().ok()?;
    let d = p[2].parse::<u32>().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) { return None; }
    let t = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let yy = if m < 3 { y - 1 } else { y };
    let w = (yy + yy / 4 - yy / 100 + yy / 400 + t[(m - 1) as usize] + d as i32).rem_euclid(7);
    let nombres = [
        "domingo", "lunes", "martes", "miércoles", "jueves", "viernes", "sábado",
    ];
    Some((nombres[w as usize], d, m, y))
}

fn dia_texto(f: &str) -> String {
    match partes_fecha(f) {
        Some((n, d, m, y)) => format!("{} {:02}/{:02}/{}", n, d, m, y),
        None => f.to_string(),
    }
}

#[derive(Serialize)]
struct Asistencia {
    fecha: String,
    presente: bool,
}

#[derive(Serialize)]
struct Actividad {
    name: String,
    fecha: String,
    fecha_entrega: String,
    estado: String,
    texto: String,
}

#[derive(Serialize)]
struct ClaseAlumno {
    id: String,
    materia: String,
    horario: String,
    asistencias: u32,
    sesiones: u32,
    pct: u32,
    promedio: Option<f64>,
    dias: Vec<Asistencia>,
    actividades: Vec<Actividad>,
}

fn resumen_clases(clases: &[Value], cuenta: &str, solo: Option<&str>) -> Vec<ClaseAlumno> {
    let mut out = Vec::new();
    for c in clases {
        let id = c.get("id").map(val_str).unwrap_or_default();
        if let Some(s) = solo {
            if id != s { continue; }
        }
        let inscrito = c
            .get("students")
            .and_then(|s| s.as_array())
            .map(|a| {
                a.iter()
                    .any(|x| x.get("account").map(val_str).unwrap_or_default().trim() == cuenta)
            })
            .unwrap_or(false);
        if !inscrito { continue; }

        let mut dias: Vec<Asistencia> = Vec::new();
        if let Some(att) = c.get("attendance").and_then(|a| a.as_object()) {
            let mut fechas: Vec<&String> = att.keys().collect();
            fechas.sort();
            for f in fechas {
                let presente = att
                    .get(f)
                    .and_then(|l| l.get(cuenta))
                    .and_then(|v| v.as_bool())
                    == Some(true);
                dias.push(Asistencia { fecha: dia_texto(f), presente });
            }
        }
        let sesiones = dias.len() as u32;
        let asistencias = dias.iter().filter(|d| d.presente).count() as u32;
        let pct = if sesiones == 0 { 100 } else { ((asistencias as f64 / sesiones as f64) * 100.0).round() as u32 };

        let mut suma = 0.0_f64;
        let mut actividades = Vec::new();
        if let Some(acts) = c.get("activities").and_then(|a| a.as_array()) {
            for a in acts {
                let g = a.get("grades").and_then(|g| g.get(cuenta));
                suma += g.and_then(nota_num).unwrap_or(0.0);
                let (estado, texto) = estado_actividad(g);
                let fecha_act = a.get("date")
                    .or_else(|| a.get("fecha"))
                    .map(val_str)
                    .unwrap_or_default();
                let fecha_entrega = a.get("due_date")
                    .or_else(|| a.get("dueDate"))
                    .or_else(|| a.get("fecha_entrega"))
                    .or_else(|| a.get("deadline"))
                    .or_else(|| a.get("fechaEntrega"))
                    .or_else(|| a.get("limite"))
                    .map(val_str)
                    .unwrap_or_default();

                actividades.push(Actividad {
                    name: a.get("name").map(val_str).unwrap_or_default(),
                    fecha: fecha_act,
                    fecha_entrega,
                    estado,
                    texto,
                });
            }
        }
        let promedio = if actividades.is_empty() {
            None
        } else {
            Some(((suma / actividades.len() as f64) * 10.0).round() / 10.0)
        };

        out.push(ClaseAlumno {
            id,
            materia: c.get("materia").map(val_str).unwrap_or_default(),
            horario: format!(
                "{} - {}",
                c.get("start").map(val_str).unwrap_or_default(),
                c.get("end").map(val_str).unwrap_or_default()
            ),
            asistencias,
            sesiones,
            pct,
            promedio,
            dias,
            actividades,
        });
    }
    out
}

// ───────────────────────── Generador de Reporte Académico Vertical PDF ─────────────────────────

struct Linea {
    x: f32,
    size: f32,
    bold: bool,
    text: String,
}

struct CeldaTabla {
    textos: Vec<String>,
    bold: bool,
}

struct TablaAcademica {
    encabezados: Vec<String>,
    filas: Vec<Vec<CeldaTabla>>,
    anchos: Vec<f32>,
}

enum Bloque {
    Texto(Linea),
    Tabla(TablaAcademica),
}

fn txt(v: &mut Vec<Bloque>, x: f32, size: f32, bold: bool, text: impl Into<String>) {
    v.push(Bloque::Texto(Linea { x, size, bold, text: text.into() }));
}

fn bloques_reporte(
    nombre: &str,
    cuenta: &str,
    profesor: &str,
    fecha: &str,
    clases: &[ClaseAlumno],
) -> Vec<Bloque> {
    let mut v: Vec<Bloque> = Vec::new();
    let fecha_fmt = if fecha.is_empty() { "Actual" } else { fecha };

    txt(&mut v, 36.0, 16.0, true, "The Tux's");
    txt(&mut v, 36.0, 11.0, true, "Reporte Académico Individual de Asistencia y Trabajos");
    let doc_str = if profesor.is_empty() { "Profesor Titular" } else { profesor };
    txt(&mut v, 36.0, 8.5, false, format!("Docente: {}  |  Emisión: {}", doc_str, fecha_fmt));
    txt(&mut v, 36.0, 4.0, false, "");

    txt(&mut v, 36.0, 10.0, true, format!("Estudiante: {}    |    Nº Cuenta: {}", nombre, cuenta));
    txt(&mut v, 36.0, 4.0, false, "");

    for c in clases {
        txt(&mut v, 36.0, 11.0, true, format!("Asignatura: {}", c.materia));
        let prom_txt = match c.promedio {
            Some(p) => format!("{:.1}", p),
            None => String::from("N/A"),
        };
        txt(&mut v, 36.0, 8.5, false, format!("Horario: {}  |  Asistencia: {}% ({} de {} clases)  |  Promedio: {}", c.horario, c.pct, c.asistencias, c.sesiones, prom_txt));
        txt(&mut v, 36.0, 4.0, false, "");

        // Tabla 1: Desglose de Asistencia por Día (Presente / Falta)
        txt(&mut v, 36.0, 9.5, true, "1. Desglose de Asistencia por Día (Presente / Falta):");
        if c.dias.is_empty() {
            txt(&mut v, 36.0, 8.5, false, "Sin registros de asistencia en este período.");
        } else {
            let mut filas_asis = Vec::new();
            for d in &c.dias {
                let (estado_str, es_presente) = if d.presente {
                    ("Presente (Asistió)".to_string(), true)
                } else {
                    ("Falta (No asistió)".to_string(), false)
                };
                filas_asis.push(vec![
                    CeldaTabla { textos: vec![d.fecha.clone()], bold: false },
                    CeldaTabla { textos: vec![estado_str], bold: es_presente },
                ]);
            }
            v.push(Bloque::Tabla(TablaAcademica {
                encabezados: vec!["Fecha de Sesión".into(), "Estado de Asistencia".into()],
                filas: filas_asis,
                anchos: vec![200.0, 340.0],
            }));
        }

        txt(&mut v, 36.0, 6.0, false, "");

        // Tabla 2: Trabajos, Tareas y Calificaciones (incluyendo fecha de entrega)
        txt(&mut v, 36.0, 9.5, true, "2. Trabajos, Tareas y Calificaciones:");
        if c.actividades.is_empty() {
            txt(&mut v, 36.0, 8.5, false, "Sin actividades registradas.");
        } else {
            let mut filas_acts = Vec::new();
            for a in &c.actividades {
                let nombre_con_fecha = if !a.fecha_entrega.is_empty() {
                    format!("{} (Entrega: {})", a.name, a.fecha_entrega)
                } else if !a.fecha.is_empty() {
                    format!("{} (Fecha: {})", a.name, a.fecha)
                } else {
                    a.name.clone()
                };
                filas_acts.push(vec![
                    CeldaTabla { textos: vec![nombre_con_fecha], bold: false },
                    CeldaTabla { textos: vec![a.texto.clone()], bold: true },
                ]);
            }
            v.push(Bloque::Tabla(TablaAcademica {
                encabezados: vec!["Nombre del Trabajo / Actividad".into(), "Calificación / Estatus".into()],
                filas: filas_acts,
                anchos: vec![350.0, 190.0],
            }));
        }

        txt(&mut v, 36.0, 10.0, false, "");
    }

    txt(&mut v, 36.0, 8.0, false, "Generado por The Tux's - Sistema de Gestión Escolar Inteligente");
    v
}

fn envolver(texto: &str, max: usize) -> Vec<String> {
    let mut lineas: Vec<String> = Vec::new();
    let mut actual = String::new();
    for palabra in texto.split_whitespace() {
        let largo = if actual.is_empty() {
            palabra.chars().count()
        } else {
            actual.chars().count() + 1 + palabra.chars().count()
        };
        if largo > max && !actual.is_empty() {
            lineas.push(std::mem::take(&mut actual));
        }
        if !actual.is_empty() {
            actual.push(' ');
        }
        actual.push_str(palabra);
    }
    if !actual.is_empty() || lineas.is_empty() {
        lineas.push(actual);
    }
    lineas
}

fn escapar(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for ch in s.chars() {
        let ch = match ch {
            '\u{2018}' | '\u{2019}' => '\'',
            '\u{201C}' | '\u{201D}' => '"',
            '\u{2013}' | '\u{2014}' | '\u{2022}' => '-',
            c => c,
        };
        let code = ch as u32;
        let b = if (32..=255).contains(&code) { code as u8 } else { b'?' };
        match b {
            b'(' | b')' | b'\\' => {
                out.push(b'\\');
                out.push(b);
            }
            _ => out.push(b),
        }
    }
    out
}

fn texto_pdf(pag: &mut Vec<u8>, fuente: &str, size: f32, x: f32, y: f32, texto: &str) {
    pag.extend_from_slice(
        format!("BT /{} {:.1} Tf 0 g {:.1} {:.1} Td (", fuente, size, x, y).as_bytes(),
    );
    pag.extend_from_slice(&escapar(texto));
    pag.extend_from_slice(b") Tj ET\n");
}

fn generar_pdf(bloques: &[Bloque]) -> Vec<u8> {
    const TOP: f32 = 742.0;
    const BOTTOM: f32 = 50.0;
    const ANCHO_UTIL: f32 = 540.0;

    let mut paginas: Vec<Vec<u8>> = vec![Vec::new()];
    let mut y = TOP;

    for b in bloques {
        match b {
            Bloque::Texto(l) => {
                let leading = l.size * 1.35;
                let max_chars = ((ANCHO_UTIL / (l.size * 0.52)) as usize).max(10);
                for parte in envolver(&l.text, max_chars) {
                    if y - leading < BOTTOM {
                        paginas.push(Vec::new());
                        y = TOP;
                    }
                    y -= leading;
                    if parte.is_empty() {
                        continue;
                    }
                    let pag = paginas.last_mut().unwrap();
                    texto_pdf(pag, if l.bold { "F2" } else { "F1" }, l.size, l.x, y, &parte);
                }
            }
            Bloque::Tabla(t) => {
                let h_row = 18.0;
                let total_h = h_row * (t.filas.len() + 1) as f32;
                if y - total_h < BOTTOM {
                    paginas.push(Vec::new());
                    y = TOP;
                }

                let pag = paginas.last_mut().unwrap();
                let mut cur_y = y;

                // Encabezados de tabla
                pag.extend_from_slice(format!("q 0.85 0.90 0.95 rg 36 {:.1} 540 {:.1} re f 0.6 G 0.5 w 36 {:.1} 540 {:.1} re S Q\n", cur_y - h_row, h_row, cur_y - h_row, h_row).as_bytes());
                let mut curr_x = 36.0;
                for (idx, h_text) in t.encabezados.iter().enumerate() {
                    let w = t.anchos.get(idx).cloned().unwrap_or(100.0);
                    texto_pdf(pag, "F2", 8.0, curr_x + 6.0, cur_y - 12.5, h_text);
                    curr_x += w;
                }
                cur_y -= h_row;

                // Filas de tabla
                for fila in &t.filas {
                    pag.extend_from_slice(format!("q 0.5 G 0.4 w 36 {:.1} 540 {:.1} re S Q\n", cur_y - h_row, h_row).as_bytes());
                    curr_x = 36.0;
                    for (idx, celda) in fila.iter().enumerate() {
                        let w = t.anchos.get(idx).cloned().unwrap_or(100.0);
                        let font = if celda.bold { "F2" } else { "F1" };
                        let txt_val = celda.textos.first().cloned().unwrap_or_default();
                        texto_pdf(pag, font, 7.5, curr_x + 6.0, cur_y - 12.5, &txt_val);
                        curr_x += w;
                    }
                    cur_y -= h_row;
                }
                y = cur_y - 6.0;
            }
        }
    }

    let n = paginas.len();
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 5 + 2 * i)).collect();
    let mut objs: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), n).into_bytes(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>".to_vec(),
    ];
    for (i, contenido) in paginas.iter().enumerate() {
        objs.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
                 /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> /Contents {} 0 R >>",
                6 + 2 * i
            )
            .into_bytes(),
        );
        let mut st = format!("<< /Length {} >>\nstream\n", contenido.len()).into_bytes();
        st.extend_from_slice(contenido);
        st.extend_from_slice(b"\nendstream");
        objs.push(st);
    }

    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n");
    let mut offsets: Vec<usize> = Vec::with_capacity(objs.len());
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", objs.len() + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for off in &offsets {
        out.extend_from_slice(format!("{:010} 00000 n \n", off).as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            objs.len() + 1,
            xref
        )
        .as_bytes(),
    );
    out
}