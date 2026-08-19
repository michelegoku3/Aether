/** Date helpers shared across views and modals. */

/**
 * Formats any date string as DD/MM/YYYY. Non-parseable input is returned
 * as-is (the backend already normalizes most dates to ISO `YYYY-MM-DD`).
 */
export const formatDateDDMMYYYY = (dateStr: string): string => {
  if (!dateStr) return '';
  const date = new Date(dateStr);
  if (isNaN(date.getTime())) return dateStr; // fallback: return as-is
  const dd = String(date.getDate()).padStart(2, '0');
  const mm = String(date.getMonth() + 1).padStart(2, '0');
  const yyyy = date.getFullYear();
  return `${dd}/${mm}/${yyyy}`;
};
