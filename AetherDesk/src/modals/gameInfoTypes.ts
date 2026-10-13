export interface GameInfoPrice {
  currency?: string | null;
  initialCents?: number | null;
  finalCents?: number | null;
  formattedFinal?: string | null;
  discountPercent?: number | null;
}

export interface GameInfoPlatforms {
  windows?: boolean | null;
  mac?: boolean | null;
  linux?: boolean | null;
  steamDeckCompatCategory?: number | null;
  steamOsCompatCategory?: number | null;
  steamMachineCompatCategory?: number | null;
  hasVrSupport?: boolean | null;
}

export interface GameInfoStoreCategories {
  supportedPlayerCategoryIds?: number[];
  featureCategoryIds?: number[];
  controllerCategoryIds?: number[];
}

export interface GameInfoScreenshot {
  id?: number | null;
  thumbnail?: string | null;
  full?: string | null;
  pathThumbnail?: string | null;
  pathFull?: string | null;
  path_thumbnail?: string | null;
  path_full?: string | null;
}

export interface GameInfoAppDetails {
  requiredAge?: string | null;
  isFree?: boolean | null;
  shortDescription?: string | null;
  supportedLanguages?: string | null;
  website?: string | null;
  headerImage?: string | null;
  capsuleImage?: string | null;
  background?: string | null;
  developers?: string[];
  publishers?: string[];
  genres?: string[];
  categories?: string[];
  recommendationsTotal?: number | null;
  achievementsTotal?: number | null;
  metacriticScore?: number | null;
  releaseDateText?: string | null;
  comingSoon?: boolean | null;
  drmNotice?: string | null;
  screenshots?: GameInfoScreenshot[];
}

export interface GameInfoLocal {
  installed: boolean;
  installDir?: string | null;
  libraryPath?: string | null;
  gamePath?: string | null;
  luaInstalled: boolean;
  manifestPinCount: number;
  updatesEnabled?: boolean | null;
}

export interface GameInfo {
  appId: number;
  name?: string | null;
  imageUrl?: string | null;
  storeUrl?: string | null;
  storeUrlPath?: string | null;
  kind?: string | null;
  hasManifest?: boolean | null;
  hasDenuvo?: boolean | null;
  hasNsfw?: boolean | null;
  hasDelisted?: boolean | null;
  releaseDateUnix?: number | null;
  originalReleaseDateUnix?: number | null;
  price?: GameInfoPrice | null;
  metascore?: string | null;
  controllerSupport?: string | null;
  platforms?: GameInfoPlatforms | null;
  storeCategories?: GameInfoStoreCategories | null;
  contentDescriptorIds?: number[];
  screenshots?: GameInfoScreenshot[];
  appDetails?: GameInfoAppDetails | null;
  local?: GameInfoLocal | null;
  updatedAtUnix?: number;
  storeSearchUpdatedAtUnix?: number | null;
  storeItemsUpdatedAtUnix?: number | null;
  appdetailsUpdatedAtUnix?: number | null;
  hubcapUpdatedAtUnix?: number | null;
  localUpdatedAtUnix?: number | null;
}

export interface GameInfoModalProps {
  appId: number;
  fallbackName: string;
  fallbackImageUrl?: string;
  onClose: () => void;
}
