// js/alumnos.js
import { invoke } from './api.js';

export async function simulateExcelUpload(currentClassId, classes, syncTeacherClassesFromDB, renderTabData, showAlert) {
    const fileInput = document.getElementById('excel-file-input');
    const file = fileInput.files[0];
    if (!file) {
        showAlert('Atención', 'Selecciona un archivo .xlsx, .xls o .csv primero.', 'error');
        return;
    }
    const cls = classes.find(c => c.id === currentClassId);
    if (!cls) return;

    try {
        const buffer = await file.arrayBuffer();
        const workbook = XLSX.read(buffer, { type: 'array', cellDates: true });
        const sheetName = workbook.SheetNames[0];
        const rows = XLSX.utils.sheet_to_json(workbook.Sheets[sheetName], { header: 1 });
        
        let nameIdx = -1;
        let accountIdx = -1;
        const parsedStudents = [];

        rows.forEach((row, idx) => {
            if (!row || row.length === 0) return;

            if (idx === 0 || (nameIdx === -1 && accountIdx === -1)) {
                row.forEach((cell, cIdx) => {
                    const val = String(cell || '').trim().toLowerCase();
                    if (val.includes('nombre') || val.includes('alumno')) nameIdx = cIdx;
                    if (val.includes('cuenta') || val.includes('matricula') || val.includes('control')) accountIdx = cIdx;
                });
                if (nameIdx !== -1 && accountIdx !== -1) return;
            }

            const nCol = nameIdx !== -1 ? nameIdx : 0;
            const aCol = accountIdx !== -1 ? accountIdx : 1;

            const rawCol0 = String(row[nCol] ?? '').trim();
            const rawCol1 = String(row[aCol] ?? '').trim();

            if (!rawCol0 || !rawCol1) return;

            let name = '';
            let account = '';

            if (/^\d+$/.test(rawCol0) && !/^\d+$/.test(rawCol1)) {
                account = rawCol0.replace(/\.0$/, '').trim();
                name = rawCol1.replace(/\s+/g, ' ');
            } else {
                name = rawCol0.replace(/\s+/g, ' ');
                account = rawCol1.replace(/\.0$/, '').trim();
            }

            if (account && name && !parsedStudents.some(s => s.account === account)) {
                parsedStudents.push({ account, name });
            }
        });

        if (parsedStudents.length === 0) {
            showAlert('Formato no reconocido', 'No se pudieron extraer alumnos. Verifica que las columnas correspondan al nombre y cuenta.', 'error');
            return;
        }

        const imported = await invoke('importar_alumnos_csv', {
            clase: { 
                id: cls.id, teacher_email: cls.teacher, materia: cls.materia, 
                color: cls.color, nivel: cls.nivel, days: cls.days.join(','), 
                start_time: cls.start, end_time: cls.end 
            },
            alumnos: parsedStudents
        });
        
        fileInput.value = '';
        await syncTeacherClassesFromDB();
        await renderTabData('alumnos');
        showAlert('Importación Completa', `Se importaron ${imported} alumnos exitosamente.`, 'success');
    } catch (err) {
        showAlert('Error al procesar archivo', String(err), 'error');
    }
}