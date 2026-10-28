use super::*;
use crate::player::PlayStationInput;
impl App {
    pub(super) fn radio_listing(&self) -> std::sync::Arc<Vec<radio::Station>> {
        let base = std::sync::Arc::as_ptr(&self.player.radio_base) as usize;
        let mut cache = self.player.radio_cache.borrow_mut();
        if cache.base != base || cache.custom != self.preferences.radio_stations {
            let mut all = self.player.radio_base.as_ref().clone();
            for custom in &self.preferences.radio_stations {
                if all.iter().any(|station| station.url == custom.url) {
                    continue;
                }
                all.push(radio::Station {
                    name: custom.name.clone(),
                    url: custom.url.clone(),
                    country: String::new(),
                    language: String::new(),
                    category: if custom.category.is_empty() {
                        "custom".into()
                    } else {
                        custom.category.clone()
                    },
                    homepage: String::new(),
                    icon: custom.icon.clone(),
                });
            }
            cache.all = std::sync::Arc::new(all);
            cache.custom.clone_from(&self.preferences.radio_stations);
            cache.base = base;
        }
        std::sync::Arc::clone(&cache.all)
    }
    pub(super) fn radio_visible(&self) -> std::sync::Arc<Vec<radio::Station>> {
        let all = self.radio_listing();
        let base = std::sync::Arc::as_ptr(&all) as usize;
        let mut cache = self.player.radio_cache.borrow_mut();
        if cache.visible_base != base
            || cache.query != self.player.radio_query
            || cache.category != self.player.radio_category
        {
            cache.visible = std::sync::Arc::new(
                all.iter()
                    .filter(|station| {
                        radio::matches_filter(
                            station,
                            &self.player.radio_query,
                            &self.player.radio_category,
                        )
                    })
                    .cloned()
                    .collect(),
            );
            cache.visible_base = base;
            cache.query.clone_from(&self.player.radio_query);
            cache.category.clone_from(&self.player.radio_category);
        }
        std::sync::Arc::clone(&cache.visible)
    }

    pub(super) fn play_radio(&mut self, project: &str, station: radio::Station) {
        let listing = self.radio_listing();
        let index = listing
            .iter()
            .position(|item| item.url == station.url)
            .unwrap_or(0);
        self.player.play_station(PlayStationInput {
            project,
            name: station.name,
            url: station.url,
            index,
        });
    }

    #[cfg(test)]
    pub(crate) fn play_station_at(&mut self, project: &str, index: usize) {
        let stations = self.radio_visible();
        let Some(station) = stations.get(index).cloned() else {
            return;
        };
        self.play_radio(project, station);
    }

    pub(crate) fn play_station_offset(&mut self, project: &str, delta: isize) {
        let stations = self.radio_visible();
        if stations.is_empty() {
            return;
        }
        let listing = self.radio_listing();
        let current = self
            .player
            .station_index
            .and_then(|index| listing.get(index).map(|station| station.url.as_str()));
        let pos = current
            .and_then(|url| stations.iter().position(|station| station.url == url))
            .unwrap_or(0);
        let len = stations.len().cast_signed();
        let Some(sum) = pos.cast_signed().checked_add(delta) else {
            return;
        };
        if len <= 0 {
            return;
        }
        let next = sum.rem_euclid(len).cast_unsigned();
        let Some(station) = stations.get(next).cloned() else {
            return;
        };
        self.play_radio(project, station);
    }
}
