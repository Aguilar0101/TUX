// js/tareas.js
import { invoke } from './api.js';

export async function executeAddActivity(currentClassId, currentRubric, syncTeacherClassesFromDB, renderTabData, showAlert) {
    const name = document.getElementById('add-act-name').value.trim();
    const type = document.getElementById('add-act-type').value;
    if (!name) {
        showAlert('Atención', 'Ingresa el título de la tarea.', 'error');
        return;
    }
    try {
        await invoke('crear_actividad', {
            id: Date.now().toString(),
            classId: currentClassId,
            name,
            actType: type,
            rubricData: type === 'rubrica' ? JSON.stringify(currentRubric) : null
        });
        await syncTeacherClassesFromDB();
        document.getElementById('add-act-name').value = '';
        await renderTabData('tareas');
        showAlert('Éxito', 'Tarea creada correctamente.', 'success');
    } catch (error) {
        showAlert('Error', String(error), 'error');
    }
}