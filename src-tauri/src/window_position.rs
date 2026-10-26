//! 主窗口的物理坐标恢复，不依赖主屏幕原点或缩放比例。
use tauri::{PhysicalPosition, PhysicalSize};

type WorkArea = (PhysicalPosition<i32>, PhysicalSize<u32>);

pub(crate) fn visible_position(
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    areas: &[WorkArea],
) -> PhysicalPosition<i32> {
    areas
        .iter()
        .filter(|(_, area_size)| area_size.width > 0 && area_size.height > 0)
        .map(|(origin, area_size)| {
            // 大于工作区的窗口仍保留左上角可达；所有运算使用宽整数。
            let max_x = (i64::from(origin.x)
                + i64::from(area_size.width.saturating_sub(size.width)))
            .min(i64::from(i32::MAX));
            let max_y = (i64::from(origin.y)
                + i64::from(area_size.height.saturating_sub(size.height)))
            .min(i64::from(i32::MAX));
            PhysicalPosition::new(
                i64::from(position.x).clamp(i64::from(origin.x), max_x) as i32,
                i64::from(position.y).clamp(i64::from(origin.y), max_y) as i32,
            )
        })
        .min_by_key(|candidate| {
            let dx = i128::from(candidate.x) - i128::from(position.x);
            let dy = i128::from(candidate.y) - i128::from(position.y);
            dx * dx + dy * dy
        })
        .unwrap_or(position)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovers_saved_position_after_removing_a_monitor() {
        let area = (PhysicalPosition::new(0, 0), PhysicalSize::new(2560, 1392));
        assert_eq!(
            visible_position(
                PhysicalPosition::new(2832, 741),
                PhysicalSize::new(496, 599),
                &[area]
            ),
            PhysicalPosition::new(2064, 741)
        );
    }

    #[test]
    fn preserves_valid_negative_secondary_monitor_coordinates() {
        let areas = [
            (PhysicalPosition::new(0, 0), PhysicalSize::new(2560, 1392)),
            (
                PhysicalPosition::new(-1920, 0),
                PhysicalSize::new(1920, 1040),
            ),
        ];
        let position = PhysicalPosition::new(-1800, 200);
        assert_eq!(
            visible_position(position, PhysicalSize::new(496, 599), &areas),
            position
        );
    }

    #[test]
    fn keeps_title_bar_accessible_when_window_exceeds_work_area() {
        let area = (PhysicalPosition::new(100, 50), PhysicalSize::new(800, 600));
        assert_eq!(
            visible_position(
                PhysicalPosition::new(1000, 900),
                PhysicalSize::new(1200, 900),
                &[area]
            ),
            PhysicalPosition::new(100, 50)
        );
    }

    #[test]
    fn missing_monitors_leave_position_unchanged() {
        let position = PhysicalPosition::new(300, 200);
        assert_eq!(
            visible_position(position, PhysicalSize::new(496, 599), &[]),
            position
        );
    }
}
