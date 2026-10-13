//! Steam-to-unified mapping helpers: price, metascore, platforms and
//! categories extracted from the two Steam metadata shapes (`SteamStoreItem`
//! from search, `StoreItemMeta` from the GetItems batch). Pure functions used
//! by `StoreService`; no network, no state.
use crate::game_info::model::{GameInfoPlatforms, GameInfoPrice, GameInfoStoreCategories};
use crate::steam::store::SteamStoreItem;
use crate::steam::store_items;

pub(super) fn price_from_store_item(item: &SteamStoreItem) -> Option<GameInfoPrice> {
    item.price.as_ref().map(|price| GameInfoPrice {
        currency: price.currency.clone(),
        initial_cents: price.initial,
        final_cents: price.final_price,
        formatted_final: price
            .final_price
            .map(|final_price| format_price_fallback(final_price, price.currency.as_deref())),
        discount_percent: price.discount_percent,
    })
}

pub(super) fn price_from_store_meta(meta: &store_items::StoreItemMeta) -> Option<GameInfoPrice> {
    meta.best_purchase_option
        .as_ref()
        .map(|price| GameInfoPrice {
            currency: None,
            initial_cents: None,
            final_cents: price
                .final_price_in_cents
                .as_ref()
                .and_then(|value| value.parse::<i64>().ok()),
            formatted_final: price.formatted_final_price.clone(),
            discount_percent: None,
        })
}

pub(super) fn format_price_fallback(amount_minor: i64, currency: Option<&str>) -> String {
    match currency.unwrap_or("EUR").to_uppercase().as_str() {
        "USD" => format!("${:.2}", amount_minor as f64 / 100.0),
        "JPY" => format!("¥{}", amount_minor),
        "EUR" => format!("€{:.2}", amount_minor as f64 / 100.0),
        other => format!("{} {:.2}", other, amount_minor as f64 / 100.0),
    }
}

pub(super) fn metascore_from_store_item(item: &SteamStoreItem) -> Option<String> {
    item.metascore.as_ref().and_then(|value| match value {
        serde_json::Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

pub(super) fn platforms_from_store_item(item: &SteamStoreItem) -> Option<GameInfoPlatforms> {
    item.platforms.as_ref().map(|platforms| GameInfoPlatforms {
        windows: platforms.windows,
        mac: platforms.mac,
        linux: platforms.linux,
        steam_deck_compat_category: None,
        steam_os_compat_category: None,
        steam_machine_compat_category: None,
        has_vr_support: None,
    })
}

pub(super) fn platforms_from_store_meta(meta: &store_items::StoreItemMeta) -> GameInfoPlatforms {
    GameInfoPlatforms {
        windows: meta.platforms.windows,
        mac: meta.platforms.mac,
        linux: meta.platforms.linux,
        steam_deck_compat_category: meta.platforms.steam_deck_compat_category,
        steam_os_compat_category: meta.platforms.steam_os_compat_category,
        steam_machine_compat_category: meta.platforms.steam_machine_compat_category,
        has_vr_support: meta.platforms.has_vr_support,
    }
}

pub(super) fn categories_from_store_meta(meta: &store_items::StoreItemMeta) -> GameInfoStoreCategories {
    GameInfoStoreCategories {
        supported_player_category_ids: meta.categories.supported_player_category_ids.clone(),
        feature_category_ids: meta.categories.feature_category_ids.clone(),
        controller_category_ids: meta.categories.controller_category_ids.clone(),
    }
}
