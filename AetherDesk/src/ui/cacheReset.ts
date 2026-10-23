import { clearCachedCovers } from './coverCache';
import { clearCachedHeroes } from './GameHeroImage';

/**
 * Clears the browser-side caches that mirror backend data. The backend half is
 * `clear_app_caches`; both must run together or the UI keeps serving covers
 * the user asked to forget.
 */
export const clearFrontendCaches = () => {
  clearCachedCovers();
  clearCachedHeroes();
};
