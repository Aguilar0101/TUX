// js/reportes.js
import { saveBinaryFile } from './api.js';

export async function exportReportToExcel(cls, showAlert) {
    if (!cls || !cls.students || cls.students.length === 0) {
        showAlert('Atención', 'No hay datos para exportar.', 'error');
        return;
    }
    const wb = XLSX.utils.book_new();
    const dates = Object.keys(cls.attendance || {}).sort();
    const acts = cls.activities || [];

    const summaryRows = [['Alumno', 'Matrícula', 'Asistencias', 'Total Sesiones', 'Promedio']];
    cls.students.forEach(st => {
        let asist = 0;
        dates.forEach(d => { if (cls.attendance[d]?.[st.account]) asist++; });
        let sum = 0, count = 0;
        acts.forEach(a => {
            const g = a.grades?.[st.account];
            if (g !== undefined && !isNaN(parseFloat(g))) { sum += parseFloat(g); count++; }
        });
        summaryRows.push([st.name, st.account, asist, dates.length, count ? (sum / count).toFixed(1) : 0]);
    });
    XLSX.utils.book_append_sheet(wb, XLSX.utils.aoa_to_sheet(summaryRows), 'Resumen General');

    try {
        const out = XLSX.write(wb, { bookType: 'xlsx', type: 'array' });
        const fname = `Reporte_${cls.materia.replace(/\s+/g, '_')}.xlsx`;
        const path = await saveBinaryFile(fname, out, 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet');
        showAlert('Éxito', `Excel generado correctamente.\n${path}`, 'success');
    } catch (e) {
        showAlert('Error', String(e), 'error');
    }
}