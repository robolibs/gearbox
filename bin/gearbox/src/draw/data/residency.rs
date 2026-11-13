use gearbox_api::draw_data::{CpuBudget, MemoryClass, StorageError};

use super::super::super::cloud::invalid;
use super::{List, lod};

pub(super) struct Page {
    pub key: lod::PageKey,
    pub coarse_bytes: u64,
    pub full_bytes: u64,
    pub full: bool,
}

#[derive(Clone, Copy)]
pub(super) struct Demand {
    pub view: usize,
    pub page: usize,
    pub error: f64,
}

pub(super) struct Plan {
    pub pages: List<Page>,
    pub bytes: u64,
}

#[derive(Default)]
struct View {
    cursor: usize,
    end: usize,
    awarded: u64,
}

pub(super) fn headroom(
    points: u32,
    coarse_points: impl Iterator<Item = u64>,
    styles: usize,
    upload_window: u64,
    auxiliary: u64,
    other: u64,
) -> u64 {
    coarse_points
        .fold(0u64, |bytes, coarse| {
            bytes.saturating_add(
                (u64::from(points) + coarse.min(u64::from(points))).saturating_mul(4),
            )
        })
        .saturating_add((styles as u64).saturating_mul(16 * 1024))
        .saturating_add(upload_window.saturating_mul(2))
        .saturating_add(auxiliary)
        .saturating_add(other)
}

pub(super) fn plan(
    budget: &CpuBudget,
    mut pages: List<Page>,
    mut demands: List<Demand>,
    view_count: usize,
    ceiling: u64,
) -> Result<Plan, StorageError> {
    if pages.windows(2).any(|pair| pair[0].key >= pair[1].key)
        || demands.iter().any(|demand| {
            demand.view >= view_count
                || demand.page >= pages.len()
                || demand.error.is_nan()
                || demand.error < 0.0
        })
    {
        return Err(invalid("invalid residency demand"));
    }
    let mut members = List::new(budget, demands.len(), MemoryClass::Scratch)?;
    for demand in &demands {
        members.push((demand.page, demand.view))?;
    }
    members.sort_unstable();
    if members.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(invalid("duplicate residency demand"));
    }
    let mut bytes = 0u64;
    for page in &mut pages {
        page.full = false;
        bytes = bytes
            .checked_add(page.coarse_bytes)
            .ok_or(StorageError::Allocation)?;
    }
    if bytes > ceiling {
        return Err(invalid("coarse residency exceeds admission ceiling"));
    }
    let density = |demand: &Demand| demand.error / pages[demand.page].full_bytes.max(1) as f64;
    demands.sort_unstable_by(|a, b| {
        a.view
            .cmp(&b.view)
            .then_with(|| density(b).total_cmp(&density(a)))
            .then_with(|| b.error.total_cmp(&a.error))
            .then_with(|| pages[a.page].key.cmp(&pages[b.page].key))
    });
    let mut views = List::new(budget, view_count, MemoryClass::Scratch)?;
    let mut cursor = 0;
    for view in 0..view_count {
        let end = cursor + demands[cursor..].partition_point(|demand| demand.view == view);
        views.push(View {
            cursor,
            end,
            awarded: 0,
        })?;
        cursor = end;
    }
    loop {
        let mut chosen = None;
        for (index, view) in views.iter_mut().enumerate() {
            while view.cursor < view.end {
                let page = &pages[demands[view.cursor].page];
                if !page.full && page.full_bytes <= ceiling - bytes {
                    break;
                }
                view.cursor += 1;
            }
            if view.cursor < view.end && chosen.is_none_or(|(_, awarded)| view.awarded < awarded) {
                chosen = Some((index, view.awarded));
            }
        }
        let Some((view, _)) = chosen else { break };
        let index = demands[views[view].cursor].page;
        let page = &mut pages[index];
        page.full = true;
        bytes += page.full_bytes;
        let start = members.partition_point(|(page, _)| *page < index);
        for (_, view) in members[start..]
            .iter()
            .take_while(|(page, _)| *page == index)
        {
            views[*view].awarded = views[*view].awarded.saturating_add(page.full_bytes);
        }
    }
    Ok(Plan { pages, bytes })
}

#[cfg(test)]
#[path = "residency_tests.rs"]
mod tests;
