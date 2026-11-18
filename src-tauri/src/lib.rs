use rusqlite::{params, Connection, Result};
use std::collections::HashMap;
use std::path::PathBuf;
use tauri::{command, AppHandle, Manager};
use rand::Rng;
use tauri_plugin_opener::OpenerExt;

mod server;

use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Message, SmtpTransport, Transport};

#[derive(serde::Serialize, serde::Deserialize)]
pub struct UserSession {
    pub email: String,
    pub name: String,
    pub role: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct SecurityQuestionInfo {
    pub email: String,
    pub question: String,
    pub has_custom_question: bool,
    pub debug_pin: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct SmtpConfig {
    pub host: String,
    pub user: String,
    pub pass: String,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct StudentData {
    pub account: String,
    pub name: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ClassSyncData {
    pub id: String,
    pub teacher_email: String,
    pub materia: String,
    pub color: String,
    pub nivel: String,
    pub days: String,
    pub start_time: String,
    pub end_time: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ActivityFullData {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub act_type: String,
    pub rubric_data: Option<String>,
    pub grades: HashMap<String, String>,
    pub rubric_grades: HashMap<String, String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct FullClassData {
    pub id: String,
    pub teacher: String,
    pub materia: String,
    pub color: String,
    pub nivel: String,
    pub days: Vec<String>,
    pub start: String,
    pub end: String,
    pub students: Vec<StudentData>,
    pub activities: Vec<ActivityFullData>,
    pub attendance: HashMap<String, HashMap<String, bool>>,
}

fn get_db_path(app: &AppHandle) -> std::result::Result<PathBuf, String> {
    let mut path = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    path.push("the_tuxs.db");
    Ok(path)
}

fn open_conn(app: &AppHandle) -> std::result::Result<Connection, String> {
    let db_path = get_db_path(app)?;
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(|e| e.to_string())?;
    Ok(conn)
}

fn init_db(db_path: &PathBuf) -> Result<()> {
    let conn = Connection::open(db_path)?;

    conn.execute_batch(
        r#"
        PRAGMA foreign_keys = ON;
        BEGIN;

        CREATE TABLE IF NOT EXISTS users (
            email TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            password TEXT NOT NULL,
            security_question TEXT NOT NULL DEFAULT '',
            security_answer TEXT NOT NULL DEFAULT '',
            reset_pin TEXT DEFAULT NULL
        );

        CREATE TABLE IF NOT EXISTS smtp_settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            host TEXT NOT NULL DEFAULT 'smtp.gmail.com',
            user TEXT NOT NULL DEFAULT '',
            pass TEXT NOT NULL DEFAULT ''
        );

        INSERT OR IGNORE INTO smtp_settings (id, host, user, pass)
        VALUES (1, 'smtp.gmail.com', '', '');

        CREATE TABLE IF NOT EXISTS classes (
            id TEXT PRIMARY KEY,
            teacher_email TEXT NOT NULL,
            materia TEXT NOT NULL,
            color TEXT NOT NULL,
            nivel TEXT NOT NULL,
            days TEXT NOT NULL,
            start_time TEXT NOT NULL,
            end_time TEXT NOT NULL,
            FOREIGN KEY (teacher_email) REFERENCES users(email) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS students (
            account TEXT NOT NULL,
            name TEXT NOT NULL,
            class_id TEXT NOT NULL,
            PRIMARY KEY (account, class_id),
            FOREIGN KEY (class_id) REFERENCES classes(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS activities (
            id TEXT PRIMARY KEY,
            class_id TEXT NOT NULL,
            name TEXT NOT NULL,
            type TEXT NOT NULL,
            rubric_data TEXT,
            FOREIGN KEY (class_id) REFERENCES classes(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS attendance (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            class_id TEXT NOT NULL,
            student_account TEXT NOT NULL,
            date TEXT NOT NULL,
            status BOOLEAN NOT NULL,
            UNIQUE(class_id, student_account, date),
            FOREIGN KEY (class_id) REFERENCES classes(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS grades (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            activity_id TEXT NOT NULL,
            student_account TEXT NOT NULL,
            grade_value TEXT NOT NULL,
            UNIQUE(activity_id, student_account),
            FOREIGN KEY (activity_id) REFERENCES activities(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS rubric_scores (
            activity_id TEXT NOT NULL,
            student_account TEXT NOT NULL,
            scores_json TEXT NOT NULL,
            PRIMARY KEY (activity_id, student_account),
            FOREIGN KEY (activity_id) REFERENCES activities(id) ON DELETE CASCADE
        );

        COMMIT;
        "#,
    )?;

    let _ = conn.execute("ALTER TABLE users ADD COLUMN security_question TEXT NOT NULL DEFAULT ''", []);
    let _ = conn.execute("ALTER TABLE users ADD COLUMN security_answer TEXT NOT NULL DEFAULT ''", []);
    let _ = conn.execute("ALTER TABLE users ADD COLUMN reset_pin TEXT DEFAULT NULL", []);

    Ok(())
}

fn is_corrupted_text(s: &str) -> bool {
    s.contains("PK")
        || s.contains("xl/")
        || s.contains('\u{FFFD}')
        || s.chars().any(|c| c.is_control())
}

fn load_smtp_config(conn: &Connection) -> SmtpConfig {
    let db_cfg = conn.query_row(
        "SELECT host, user, pass FROM smtp_settings WHERE id = 1",
        [],
        |r| {
            Ok(SmtpConfig {
                host: r.get(0)?,
                user: r.get(1)?,
                pass: r.get(2)?,
            })
        },
    );

    let mut cfg = db_cfg.unwrap_or(SmtpConfig {
        host: "smtp.gmail.com".to_string(),
        user: String::new(),
        pass: String::new(),
    });

    if cfg.user.trim().is_empty() {
        if let Ok(env_user) = std::env::var("SMTP_USER") {
            cfg.user = env_user;
        }
    }
    if cfg.pass.trim().is_empty() {
        if let Ok(env_pass) = std::env::var("SMTP_PASS") {
            cfg.pass = env_pass;
        }
    }
    if let Ok(env_host) = std::env::var("SMTP_HOST") {
        if !env_host.trim().is_empty() {
            cfg.host = env_host;
        }
    }
    cfg
}

fn enviar_pin_correo(cfg: &SmtpConfig, destinatario: &str, nombre: &str, pin: &str) -> std::result::Result<(), String> {
    let smtp_host = cfg.host.trim();
    let smtp_user = cfg.user.trim();
    let smtp_pass = cfg.pass.trim();

    if smtp_user.is_empty() || smtp_pass.is_empty() {
        return Err("No hay correo SMTP configurado. Configúralo en Configuración Docente > Correo / SMTP o usa variables de entorno SMTP_USER y SMTP_PASS.".to_string());
    }

    let parsed_from = format!("The Tux's Soporte <{}>", smtp_user)
        .parse()
        .map_err(|e: lettre::address::AddressError| e.to_string())?;

    let parsed_to = format!("{} <{}>", nombre, destinatario)
        .parse()
        .map_err(|e: lettre::address::AddressError| e.to_string())?;

    let email = Message::builder()
        .from(parsed_from)
        .to(parsed_to)
        .subject("The Tux's — Código PIN de Verificación")
        .header(ContentType::TEXT_PLAIN)
        .body(format!(
            "Hola {},\n\nTu código PIN de verificación para restablecer tu contraseña en The Tux's es: {}\n\nIntroduce este código de 6 dígitos junto con tu respuesta de seguridad en la aplicación.",
            nombre, pin
        ))
        .map_err(|e| e.to_string())?;

    let creds = Credentials::new(smtp_user.to_owned(), smtp_pass.to_owned());

    let mailer = SmtpTransport::starttls_relay(smtp_host)
        .map_err(|e| format!("Error al conectar con el servidor SMTP ({}): {}", smtp_host, e))?
        .port(587)
        .credentials(creds)
        .build();

    mailer
        .send(&email)
        .map_err(|e| format!("No se pudo enviar el correo SMTP: {}", e))?;

    Ok(())
}

#[command]
fn guardar_config_smtp(
    app: AppHandle,
    host: String,
    user: String,
    pass: String,
) -> std::result::Result<(), String> {
    let conn = open_conn(&app)?;
    conn.execute(
        "INSERT INTO smtp_settings (id, host, user, pass) VALUES (1, ?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET host = excluded.host, user = excluded.user, pass = excluded.pass",
        params![host.trim(), user.trim(), pass.trim()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[command]
fn obtener_config_smtp(app: AppHandle) -> std::result::Result<SmtpConfig, String> {
    let conn = open_conn(&app)?;
    Ok(load_smtp_config(&conn))
}

#[command]
fn registrar_profesor(
    app: AppHandle,
    name: String,
    email: String,
    password: String,
    security_question: Option<String>,
    security_answer: Option<String>,
) -> std::result::Result<(), String> {
    let conn = open_conn(&app)?;
    let q = security_question.unwrap_or_default().trim().to_string();
    let a = security_answer.unwrap_or_default().trim().to_lowercase();

    conn.execute(
        "INSERT INTO users (email, name, password, security_question, security_answer) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![email.trim(), name.trim(), password, q, a],
    )
    .map_err(|_| "El correo ya está registrado o hubo un error al guardar.".to_string())?;
    Ok(())
}

#[command]
fn login_profesor(
    app: AppHandle,
    email: String,
    password: String,
) -> std::result::Result<UserSession, String> {
    let conn = open_conn(&app)?;
    let mut stmt = conn
        .prepare("SELECT name, password FROM users WHERE email = ?1")
        .map_err(|e| e.to_string())?;

    let result = stmt.query_row([email.trim()], |row| {
        let db_name: String = row.get(0)?;
        let db_pass: String = row.get(1)?;
        Ok((db_name, db_pass))
    });

    match result {
        Ok((name, db_pass)) => {
            if db_pass == password {
                Ok(UserSession {
                    email: email.trim().to_string(),
                    name,
                    role: "profesor".to_string(),
                })
            } else {
                Err("Contraseña incorrecta".to_string())
            }
        }
        Err(_) => Err("El usuario no está registrado".to_string()),
    }
}

#[command]
fn obtener_pregunta_seguridad(
    app: AppHandle,
    email: String,
) -> std::result::Result<SecurityQuestionInfo, String> {
    let conn = open_conn(&app)?;
    let clean_email = email.trim();
    
    let row_data: (String, String) = conn
        .query_row(
            "SELECT name, security_question FROM users WHERE email = ?1",
            [clean_email],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| "No existe ninguna cuenta de profesor registrada con ese correo.".to_string())?;

    let (db_name, q) = row_data;
    
    let mut rng = rand::thread_rng();
    let pin: u32 = rng.gen_range(100000..999999);
    let pin_str = pin.to_string();

    conn.execute(
        "UPDATE users SET reset_pin = ?1 WHERE email = ?2",
        params![pin_str, clean_email],
    )
    .map_err(|e| e.to_string())?;

    let smtp_cfg = load_smtp_config(&conn);
    enviar_pin_correo(&smtp_cfg, clean_email, &db_name, &pin_str)?;

    let trimmed_q = q.trim().to_string();
    if trimmed_q.is_empty() {
        Ok(SecurityQuestionInfo {
            email: clean_email.to_string(),
            question: "Introduce el PIN de 6 dígitos enviado a tu correo y tu Nombre Completo:".to_string(),
            has_custom_question: false,
            debug_pin: None,
        })
    } else {
        Ok(SecurityQuestionInfo {
            email: clean_email.to_string(),
            question: format!("{} (Ingresa también el PIN de 6 dígitos)", trimmed_q),
            has_custom_question: true,
            debug_pin: None,
        })
    }
}

#[command]
fn restablecer_contrasena(
    app: AppHandle,
    email: String,
    answer: String,
    pin: String,
    new_password: String,
    _new_question: Option<String>,
    _new_answer: Option<String>,
) -> std::result::Result<(), String> {
    if new_password.trim().len() < 6 {
        return Err("La nueva contraseña debe tener al menos 6 caracteres.".to_string());
    }

    let conn = open_conn(&app)?;
    let clean_email = email.trim();
    let clean_input_answer = answer.trim().to_lowercase();
    let clean_pin = pin.trim();

    let row_data: (String, String, String, Option<String>) = conn
        .query_row(
            "SELECT name, security_question, security_answer, reset_pin FROM users WHERE email = ?1",
            [clean_email],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|_| "No se encontró la cuenta del docente.".to_string())?;

    let (db_name, db_question, db_answer, db_pin) = row_data;

    if db_pin.is_none() || db_pin.as_deref() != Some(clean_pin) {
        return Err("El PIN de verificación es incorrecto o ha expirado.".to_string());
    }

    let is_valid = if db_question.trim().is_empty() {
        db_name.trim().to_lowercase() == clean_input_answer
    } else {
        !db_answer.trim().is_empty() && db_answer.trim().to_lowercase() == clean_input_answer
    };

    if !is_valid {
        return Err("La respuesta de verificación no coincide con nuestros registros.".to_string());
    }

    conn.execute(
        "UPDATE users SET password = ?1, reset_pin = NULL WHERE email = ?2",
        params![new_password, clean_email],
    )
    .map_err(|e| e.to_string())?;

    Ok(())
}

#[command]
fn actualizar_profesor(
    app: AppHandle,
    old_email: String,
    name: String,
    new_email: String,
    new_password: Option<String>,
) -> std::result::Result<UserSession, String> {
    let mut conn = open_conn(&app)?;
    let old_em = old_email.trim().to_string();
    let new_em = new_email.trim().to_string();
    let clean_name = name.trim().to_string();

    let (current_pass, sec_q, sec_a): (String, String, String) = conn
        .query_row(
            "SELECT password, security_question, security_answer FROM users WHERE email = ?1",
            [&old_em],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| "No se encontró la cuenta del docente.".to_string())?;

    let final_pass = match new_password {
        Some(p) if !p.trim().is_empty() => p,
        _ => current_pass,
    };

    if old_em == new_em {
        conn.execute(
            "UPDATE users SET name = ?1, password = ?2 WHERE email = ?3",
            params![clean_name, final_pass, old_em],
        )
        .map_err(|e| e.to_string())?;
    } else {
        conn.execute_batch("PRAGMA foreign_keys = OFF;").map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;

        tx.execute(
            "INSERT INTO users (email, name, password, security_question, security_answer) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![new_em, clean_name, final_pass, sec_q, sec_a],
        )
        .map_err(|_| "El nuevo correo ya está en uso por otra cuenta.".to_string())?;

        tx.execute(
            "UPDATE classes SET teacher_email = ?1 WHERE teacher_email = ?2",
            params![new_em, old_em],
        )
        .map_err(|e| e.to_string())?;

        tx.execute("DELETE FROM users WHERE email = ?1", params![old_em])
            .map_err(|e| e.to_string())?;

        tx.commit().map_err(|e| e.to_string())?;
        conn.execute_batch("PRAGMA foreign_keys = ON;").map_err(|e| e.to_string())?;
    }

    Ok(UserSession {
        email: new_em,
        name: clean_name,
        role: "profesor".to_string(),
    })
}

#[command]
fn login_alumno(app: AppHandle, account: String) -> std::result::Result<UserSession, String> {
    let conn = open_conn(&app)?;
    let clean_acc = account.trim();
    let mut stmt = conn
        .prepare("SELECT name, account FROM students WHERE account = ?1 LIMIT 1")
        .map_err(|e| e.to_string())?;

    let result = stmt.query_row([clean_acc], |row| {
        let name: String = row.get(0)?;
        let acc: String = row.get(1)?;
        Ok((name, acc))
    });

    match result {
        Ok((name, acc)) => Ok(UserSession {
            email: acc,
            name,
            role: "alumno".to_string(),
        }),
        Err(_) => Err("Número de cuenta no encontrado".to_string()),
    }
}

#[command]
fn guardar_clase(app: AppHandle, clase: ClassSyncData) -> std::result::Result<(), String> {
    let conn = open_conn(&app)?;
    conn.execute(
        "INSERT INTO classes (id, teacher_email, materia, color, nivel, days, start_time, end_time)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET
            materia = excluded.materia,
            color = excluded.color,
            nivel = excluded.nivel,
            days = excluded.days,
            start_time = excluded.start_time,
            end_time = excluded.end_time",
        params![
            clase.id,
            clase.teacher_email,
            clase.materia,
            clase.color,
            clase.nivel,
            clase.days,
            clase.start_time,
            clase.end_time
        ],
    )
    .map_err(|e| format!("Error al guardar la materia: {}", e))?;
    Ok(())
}

#[command]
fn eliminar_clase(app: AppHandle, class_id: String) -> std::result::Result<(), String> {
    let conn = open_conn(&app)?;
    conn.execute("DELETE FROM classes WHERE id = ?1", [class_id])
        .map_err(|e| format!("Error al eliminar la materia: {}", e))?;
    Ok(())
}

fn hydrate_class(
    conn: &Connection,
    id: String,
    teacher: String,
    materia: String,
    color: String,
    nivel: String,
    days_str: String,
    start: String,
    end: String,
) -> std::result::Result<FullClassData, String> {
    let days: Vec<String> = days_str
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let mut students = Vec::new();
    {
        let mut stmt = conn
            .prepare("SELECT account, name FROM students WHERE class_id = ?1 ORDER BY name ASC")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([&id], |r| {
                Ok(StudentData {
                    account: r.get(0)?,
                    name: r.get(1)?,
                })
            })
            .map_err(|e| e.to_string())?;
        for item in rows {
            students.push(item.map_err(|e| e.to_string())?);
        }
    }

    let mut attendance: HashMap<String, HashMap<String, bool>> = HashMap::new();
    {
        let mut stmt = conn
            .prepare("SELECT date, student_account, status FROM attendance WHERE class_id = ?1")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([&id], |r| {
                let d: String = r.get(0)?;
                let acc: String = r.get(1)?;
                let st: bool = r.get(2)?;
                Ok((d, acc, st))
            })
            .map_err(|e| e.to_string())?;
        for item in rows {
            let (d, acc, st) = item.map_err(|e| e.to_string())?;
            attendance.entry(d).or_default().insert(acc, st);
        }
    }

    let mut activities = Vec::new();
    {
        let mut stmt = conn
            .prepare("SELECT id, name, type, rubric_data FROM activities WHERE class_id = ?1")
            .map_err(|e| e.to_string())?;
        let act_rows = stmt
            .query_map([&id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?;

        let mut raw_acts = Vec::new();
        for item in act_rows {
            raw_acts.push(item.map_err(|e| e.to_string())?);
        }

        for (act_id, act_name, act_type, rubric_data) in raw_acts {
            let mut grades = HashMap::new();
            let mut g_stmt = conn
                .prepare("SELECT student_account, grade_value FROM grades WHERE activity_id = ?1")
                .map_err(|e| e.to_string())?;
            let g_rows = g_stmt
                .query_map([&act_id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(|e| e.to_string())?;
            for g in g_rows {
                let (acc, val) = g.map_err(|e| e.to_string())?;
                grades.insert(acc, val);
            }

            let mut rubric_grades = HashMap::new();
            let mut r_stmt = conn
                .prepare("SELECT student_account, scores_json FROM rubric_scores WHERE activity_id = ?1")
                .map_err(|e| e.to_string())?;
            let r_rows = r_stmt
                .query_map([&act_id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(|e| e.to_string())?;
            for rg in r_rows {
                let (acc, json_str) = rg.map_err(|e| e.to_string())?;
                rubric_grades.insert(acc, json_str);
            }

            activities.push(ActivityFullData {
                id: act_id,
                name: act_name,
                act_type,
                rubric_data,
                grades,
                rubric_grades,
            });
        }
    }

    Ok(FullClassData {
        id,
        teacher,
        materia,
        color,
        nivel,
        days,
        start,
        end,
        students,
        activities,
        attendance,
    })
}

#[command]
fn obtener_clases_profesor(
    app: AppHandle,
    teacher_email: String,
) -> std::result::Result<Vec<FullClassData>, String> {
    let conn = open_conn(&app)?;
    let mut raw_classes = Vec::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT id, teacher_email, materia, color, nivel, days, start_time, end_time
                 FROM classes WHERE teacher_email = ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([teacher_email.trim()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        for item in rows {
            raw_classes.push(item.map_err(|e| e.to_string())?);
        }
    }

    let mut result = Vec::new();
    for (id, teacher, materia, color, nivel, days_str, start, end) in raw_classes {
        result.push(hydrate_class(
            &conn, id, teacher, materia, color, nivel, days_str, start, end,
        )?);
    }
    Ok(result)
}

#[command]
fn obtener_clases_alumno(
    app: AppHandle,
    account: String,
) -> std::result::Result<Vec<FullClassData>, String> {
    let conn = open_conn(&app)?;
    let mut raw_classes = Vec::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT c.id, c.teacher_email, c.materia, c.color, c.nivel, c.days, c.start_time, c.end_time
                 FROM classes c
                 INNER JOIN students s ON s.class_id = c.id
                 WHERE s.account = ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([account.trim()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        for item in rows {
            raw_classes.push(item.map_err(|e| e.to_string())?);
        }
    }

    let mut result = Vec::new();
    for (id, teacher, materia, color, nivel, days_str, start, end) in raw_classes {
        result.push(hydrate_class(
            &conn, id, teacher, materia, color, nivel, days_str, start, end,
        )?);
    }
    Ok(result)
}

#[command]
fn importar_alumnos_csv(
    app: AppHandle,
    clase: ClassSyncData,
    alumnos: Vec<StudentData>,
) -> std::result::Result<usize, String> {
    let mut conn = open_conn(&app)?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;

    tx.execute(
        "INSERT OR IGNORE INTO classes (id, teacher_email, materia, color, nivel, days, start_time, end_time)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            clase.id,
            clase.teacher_email,
            clase.materia,
            clase.color,
            clase.nivel,
            clase.days,
            clase.start_time,
            clase.end_time
        ],
    )
    .map_err(|e| format!("Error al sincronizar la clase: {}", e))?;

    let mut importados = 0usize;
    {
        let mut stmt = tx
            .prepare("INSERT OR IGNORE INTO students (account, name, class_id) VALUES (?1, ?2, ?3)")
            .map_err(|e| e.to_string())?;

        for alumno in alumnos {
            let cuenta = alumno.account.trim();
            let nombre = alumno.name.trim();
            if !cuenta.is_empty()
                && !nombre.is_empty()
                && !is_corrupted_text(cuenta)
                && !is_corrupted_text(nombre)
            {
                let filas = stmt
                    .execute(params![cuenta, nombre, clase.id])
                    .map_err(|e| format!("Error al registrar a {}: {}", nombre, e))?;
                importados += filas;
            }
        }
    }

    tx.commit().map_err(|e| e.to_string())?;
    Ok(importados)
}

#[command]
fn obtener_alumnos_clase(
    app: AppHandle,
    class_id: String,
) -> std::result::Result<Vec<StudentData>, String> {
    let conn = open_conn(&app)?;
    let mut stmt = conn
        .prepare("SELECT account, name FROM students WHERE class_id = ?1 ORDER BY name ASC")
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([&class_id], |row| {
            Ok(StudentData {
                account: row.get(0)?,
                name: row.get(1)?,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut lista = Vec::new();
    for item in rows {
        lista.push(item.map_err(|e| e.to_string())?);
    }
    Ok(lista)
}

#[command]
fn eliminar_alumno(
    app: AppHandle,
    class_id: String,
    account: String,
) -> std::result::Result<(), String> {
    let mut conn = open_conn(&app)?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;

    tx.execute(
        "DELETE FROM attendance WHERE class_id = ?1 AND student_account = ?2",
        params![class_id, account],
    )
    .map_err(|e| e.to_string())?;

    tx.execute(
        "DELETE FROM grades WHERE student_account = ?1 AND activity_id IN (SELECT id FROM activities WHERE class_id = ?2)",
        params![account, class_id],
    )
    .map_err(|e| e.to_string())?;

    tx.execute(
        "DELETE FROM rubric_scores WHERE student_account = ?1 AND activity_id IN (SELECT id FROM activities WHERE class_id = ?2)",
        params![account, class_id],
    )
    .map_err(|e| e.to_string())?;

    tx.execute(
        "DELETE FROM students WHERE class_id = ?1 AND account = ?2",
        params![class_id, account],
    )
    .map_err(|e| e.to_string())?;

    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

#[command]
fn inicializar_asistencia_fecha(
    app: AppHandle,
    class_id: String,
    date: String,
) -> std::result::Result<(), String> {
    let mut conn = open_conn(&app)?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;

    let mut accounts = Vec::new();
    {
        let mut stmt = tx
            .prepare("SELECT account FROM students WHERE class_id = ?1")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([&class_id], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        for r in rows {
            accounts.push(r.map_err(|e| e.to_string())?);
        }
    }

    {
        let mut ins = tx
            .prepare(
                "INSERT OR IGNORE INTO attendance (class_id, student_account, date, status)
                 VALUES (?1, ?2, ?3, 0)",
            )
            .map_err(|e| e.to_string())?;
        for acc in accounts {
            ins.execute(params![class_id, acc, date])
                .map_err(|e| e.to_string())?;
        }
    }

    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

#[command]
fn registrar_asistencia(
    app: AppHandle,
    class_id: String,
    student_account: String,
    date: String,
    status: bool,
) -> std::result::Result<(), String> {
    let conn = open_conn(&app)?;
    conn.execute(
        "INSERT INTO attendance (class_id, student_account, date, status)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(class_id, student_account, date) DO UPDATE SET status = excluded.status",
        params![class_id, student_account, date, status],
    )
    .map_err(|e| format!("Error al registrar asistencia: {}", e))?;
    Ok(())
}

#[command]
fn crear_actividad(
    app: AppHandle,
    id: String,
    class_id: String,
    name: String,
    act_type: String,
    rubric_data: Option<String>,
) -> std::result::Result<(), String> {
    let conn = open_conn(&app)?;
    conn.execute(
        "INSERT INTO activities (id, class_id, name, type, rubric_data) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, class_id, name.trim(), act_type, rubric_data],
    )
    .map_err(|e| format!("Error al crear actividad: {}", e))?;
    Ok(())
}

#[command]
fn editar_actividad(
    app: AppHandle,
    id: String,
    name: String,
    act_type: String,
) -> std::result::Result<(), String> {
    let conn = open_conn(&app)?;
    conn.execute(
        "UPDATE activities SET name = ?1, type = ?2 WHERE id = ?3",
        params![name.trim(), act_type, id],
    )
    .map_err(|e| format!("Error al editar actividad: {}", e))?;
    Ok(())
}

#[command]
fn eliminar_actividad(app: AppHandle, id: String) -> std::result::Result<(), String> {
    let conn = open_conn(&app)?;
    conn.execute("DELETE FROM activities WHERE id = ?1", [id])
        .map_err(|e| format!("Error al eliminar actividad: {}", e))?;
    Ok(())
}

#[command]
fn guardar_calificacion(
    app: AppHandle,
    activity_id: String,
    student_account: String,
    grade_value: String,
    rubric_scores_json: Option<String>,
) -> std::result::Result<(), String> {
    let mut conn = open_conn(&app)?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;

    tx.execute(
        "INSERT INTO grades (activity_id, student_account, grade_value)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(activity_id, student_account) DO UPDATE SET grade_value = excluded.grade_value",
        params![activity_id, student_account, grade_value],
    )
    .map_err(|e| format!("Error al guardar calificación: {}", e))?;

    if let Some(json_str) = rubric_scores_json {
        tx.execute(
            "INSERT INTO rubric_scores (activity_id, student_account, scores_json)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(activity_id, student_account) DO UPDATE SET scores_json = excluded.scores_json",
            params![activity_id, student_account, json_str],
        )
        .map_err(|e| format!("Error al guardar desglose de rúbrica: {}", e))?;
    }

    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

/// Guarda un archivo generado en el frontend (PDF, XLSX, PNG) en Descargas/TheTuxs
/// y lo abre con la aplicación predeterminada del sistema.
#[command]
fn guardar_archivo(
    app: AppHandle,
    filename: String,
    bytes: Vec<u8>,
) -> std::result::Result<String, String> {
    let safe: String = filename
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '.' || c == '_' || c == '-' { c } else { '_' })
        .collect();
    if safe.is_empty() || safe.starts_with('.') {
        return Err("Nombre de archivo no válido".to_string());
    }

    // 1) Descargas/TheTuxs  2) Documentos/TheTuxs  3) carpeta privada de la app (Android sin permiso)
    let mut candidatos: Vec<PathBuf> = Vec::new();
    if let Ok(d) = app.path().download_dir() { candidatos.push(d.join("TheTuxs")); }
    if let Ok(d) = app.path().document_dir() { candidatos.push(d.join("TheTuxs")); }
    if let Ok(d) = app.path().app_data_dir() { candidatos.push(d.join("reportes")); }

    let mut guardado: Option<PathBuf> = None;
    let mut ultimo_error = String::from("No hay carpeta disponible para guardar");
    for dir in candidatos {
        if let Err(e) = std::fs::create_dir_all(&dir) { ultimo_error = e.to_string(); continue; }
        let destino = dir.join(&safe);
        match std::fs::write(&destino, &bytes) {
            Ok(_) => { guardado = Some(destino); break; }
            Err(e) => ultimo_error = e.to_string(),
        }
    }
    let path = guardado.ok_or_else(|| format!("No se pudo guardar el archivo: {}", ultimo_error))?;

    let path_str = path.to_string_lossy().to_string();
    let _ = app.opener().open_path(path_str.clone(), None::<&str>);
    Ok(path_str)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(server::ServerState::default())
        .setup(|app| {
            let db_path = get_db_path(app.handle())
                .expect("No se pudo obtener la ruta de datos de la aplicación");
            init_db(&db_path).expect("Error crítico al inicializar SQLite");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            guardar_config_smtp,
            obtener_config_smtp,
            registrar_profesor,
            login_profesor,
            obtener_pregunta_seguridad,
            restablecer_contrasena,
            actualizar_profesor,
            login_alumno,
            guardar_clase,
            eliminar_clase,
            obtener_clases_profesor,
            obtener_clases_alumno,
            importar_alumnos_csv,
            obtener_alumnos_clase,
            eliminar_alumno,
            inicializar_asistencia_fecha,
            registrar_asistencia,
            crear_actividad,
            editar_actividad,
            eliminar_actividad,
            guardar_calificacion,
            guardar_archivo,
            server::servidor_alumnos_iniciar,
            server::servidor_alumnos_actualizar,
            server::servidor_alumnos_detener,
            server::servidor_alumnos_estado
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}