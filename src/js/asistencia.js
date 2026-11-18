// js/asistencia.js
import { invoke, localDateStr } from './api.js';

export async function addTodayAttendanceColumn(currentClassId, classes, syncTeacherClassesFromDB, renderTabData, showAlert) {
    const cls = classes.find(c => c.id === currentClassId);
    if (!cls || !cls.students || cls.students.length === 0) {
        showAlert('Atención', 'Agrega alumnos primero.', 'error');
        return;
    }
    const today = localDateStr();
    try {
        await invoke('inicializar_asistencia_fecha', { classId: currentClassId, date: today });
        await syncTeacherClassesFromDB();
        await renderTabData('asistencia');
        showAlert('Listo', `Columna de asistencia (${today}) agregada.`, 'success');
    } catch (error) {
        showAlert('Error', String(error), 'error');
    }
}

export async function executeToggleMatrix(currentClassId, classes, syncTeacherClassesFromDB, renderTabData, date, accountId, showAlert) {
    const cls = classes.find(c => c.id === currentClassId);
    if (!cls) return;
    const currentVal = cls.attendance?.[date]?.[accountId] === true;
    try {
        await invoke('registrar_asistencia', {
            classId: currentClassId,
            studentAccount: accountId,
            date,
            status: !currentVal
        });
        await syncTeacherClassesFromDB();
        await renderTabData('asistencia');
    } catch (error) {
        showAlert('Error', String(error), 'error');
    }
}