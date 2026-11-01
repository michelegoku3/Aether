//! Steam-to-unified mapping helpers: price, metascore, platforms and
//! categories extracted from the two Steam metadata shapes (`SteamStoreItem`
//! from search, `StoreItemMeta` from the GetItems batch). Pure functions used
//! by `StoreService`; no network, no state.
use crate::core::settings::normalize_store_currency;
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

pub(super) fn price_from_store_meta(
    meta: &store_items::StoreItemMeta,
    store_currency: &str,
) -> Option<GameInfoPrice> {
    // If the base game is free, don't return a price — the UI will show
    // "Free to Play" instead. A paid upgrade (e.g. CS2 Prime Status) is not
    // the game's price.
    if meta.is_free {
        return None;
    }
    meta.best_purchase_option
        .as_ref()
        .map(|price| GameInfoPrice {
            // GetItems only formats the string; carry the requested currency
            // so the UI never falls back to a guessed symbol.
            currency: Some(normalize_store_currency(store_currency).to_uppercase()),
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
    match currency.unwrap_or("").to_uppercase().as_str() {
        "USD" => format!("${:.2}", amount_minor as f64 / 100.0),
        "JPY" => format!("¥{}", amount_minor),
        "EUR" => format!("€{:.2}", amount_minor as f64 / 100.0),
        // No currency at all: a bare amount is safer than inventing a symbol.
        "" => format!("{:.2}", amount_minor as f64 / 100.0),
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

pub(super) fn categories_from_store_meta(
    meta: &store_items::StoreItemMeta,
) -> GameInfoStoreCategories {
    GameInfoStoreCategories {
        supported_player_category_ids: meta.categories.supported_player_category_ids.clone(),
        feature_category_ids: meta.categories.feature_category_ids.clone(),
        controller_category_ids: meta.categories.controller_category_ids.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_price_carries_the_selected_currency() {
        let meta = store_items::StoreItemMeta {
            best_purchase_option: Some(store_items::StoreItemPurchaseOption {
                formatted_final_price: Some("Rp 150.000".to_string()),
                final_price_in_cents: Some("15000000".to_string()),
            }),
            ..Default::default()
        };

        let price = price_from_store_meta(&meta, "idr").expect("price mapped");
        assert_eq!(price.currency.as_deref(), Some("IDR"));
        assert_eq!(price.final_cents, Some(15000000));
        assert_eq!(price.formatted_final.as_deref(), Some("Rp 150.000"));
    }

    #[test]
    fn fallback_price_never_guesses_eur() {
        assert_eq!(format_price_fallback(1999, None), "19.99");
        assert_eq!(
            format_price_fallback(15000000, Some("IDR")),
            "IDR 150000.00"
        );
    }
}
