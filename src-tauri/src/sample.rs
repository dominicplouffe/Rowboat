//! The small SQLite database the onboarding wizard can open, so there is
//! something to query before a real connection exists. Generated on demand
//! rather than shipped, which keeps it out of the installer.

use std::path::Path;

use rusqlite::{params, Connection};

use crate::error::{Error, Result};

const SCHEMA: &str = "
CREATE TABLE customers (
    id         INTEGER PRIMARY KEY,
    name       TEXT NOT NULL,
    email      TEXT NOT NULL UNIQUE,
    country    TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE products (
    id       INTEGER PRIMARY KEY,
    name     TEXT NOT NULL,
    category TEXT NOT NULL,
    price    REAL NOT NULL
);
CREATE TABLE orders (
    id          INTEGER PRIMARY KEY,
    customer_id INTEGER NOT NULL REFERENCES customers (id),
    status      TEXT NOT NULL CHECK (status IN ('new', 'paid', 'shipped', 'refunded')),
    ordered_at  TEXT NOT NULL,
    details     TEXT
);
CREATE TABLE order_items (
    order_id   INTEGER NOT NULL REFERENCES orders (id),
    product_id INTEGER NOT NULL REFERENCES products (id),
    quantity   INTEGER NOT NULL,
    unit_price REAL NOT NULL,
    PRIMARY KEY (order_id, product_id)
);
CREATE INDEX orders_customer ON orders (customer_id);
CREATE INDEX orders_ordered_at ON orders (ordered_at);
CREATE VIEW order_totals AS
    SELECT o.id AS order_id, o.customer_id, o.status, o.ordered_at,
           round(sum(i.quantity * i.unit_price), 2) AS total
    FROM orders o JOIN order_items i ON i.order_id = o.id
    GROUP BY o.id;
";

const FIRST: [&str; 12] = [
    "Ada", "Grace", "Linus", "Margaret", "Dennis", "Barbara", "Ken", "Radia", "Edsger", "Frances",
    "Alan", "Hedy",
];
const LAST: [&str; 10] = [
    "Rowe", "Oar", "Keel", "Harbor", "Tide", "Marsh", "Lake", "Brook", "Reed", "Shore",
];
const COUNTRIES: [&str; 6] = ["CA", "US", "FR", "DE", "JP", "BR"];
const PRODUCTS: [(&str, &str, f64); 12] = [
    ("Oak oar", "Oars", 89.0),
    ("Carbon oar", "Oars", 249.0),
    ("Oarlock pair", "Hardware", 34.5),
    ("Bailer", "Hardware", 9.99),
    ("Life jacket", "Safety", 59.0),
    ("Whistle", "Safety", 4.5),
    ("Dry bag 20 L", "Bags", 27.0),
    ("Dry bag 40 L", "Bags", 39.0),
    ("Seat cushion", "Comfort", 22.0),
    ("Bow line 10 m", "Rope", 14.0),
    ("Anchor 3 kg", "Hardware", 45.0),
    ("Chart case", "Bags", 18.0),
];
const STATUSES: [&str; 4] = ["paid", "shipped", "paid", "refunded"];

/// A tiny deterministic generator: the sample must look the same everywhere.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, bound: i64) -> i64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) % bound as u64) as i64
    }
}

/// Creates the sample database at `path` unless it is already there.
pub fn create(path: &Path) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Build next to the target and rename, so a crash cannot leave half a file.
    let partial = path.with_extension("db.partial");
    let _ = std::fs::remove_file(&partial);
    let fill = || -> rusqlite::Result<()> {
        let mut conn = Connection::open(&partial)?;
        conn.execute_batch(SCHEMA)?;
        let tx = conn.transaction()?;
        let mut rng = Lcg(42);
        for id in 1..=120i64 {
            let (first, last) = (FIRST[rng.next(12) as usize], LAST[rng.next(10) as usize]);
            tx.execute(
                "INSERT INTO customers VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    id,
                    format!("{first} {last}"),
                    format!(
                        "{}.{}{id}@example.com",
                        first.to_lowercase(),
                        last.to_lowercase()
                    ),
                    COUNTRIES[rng.next(6) as usize],
                    format!("2025-{:02}-{:02}", 1 + rng.next(12), 1 + rng.next(28)),
                ],
            )?;
        }
        for (i, (name, category, price)) in PRODUCTS.iter().enumerate() {
            tx.execute(
                "INSERT INTO products VALUES (?1, ?2, ?3, ?4)",
                params![i as i64 + 1, name, category, price],
            )?;
        }
        for id in 1..=600i64 {
            let status = if id > 570 {
                "new"
            } else {
                STATUSES[rng.next(4) as usize]
            };
            let details = format!(
                r#"{{"channel":"{}","gift":{},"tags":["{}"]}}"#,
                ["web", "store", "phone"][rng.next(3) as usize],
                rng.next(5) == 0,
                ["spring", "regatta", "clearance"][rng.next(3) as usize],
            );
            tx.execute(
                "INSERT INTO orders VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    id,
                    1 + rng.next(120),
                    status,
                    format!(
                        "2026-{:02}-{:02} {:02}:{:02}:00",
                        1 + rng.next(10),
                        1 + rng.next(28),
                        8 + rng.next(12),
                        rng.next(60)
                    ),
                    details,
                ],
            )?;
            let first = rng.next(12);
            for k in 0..=rng.next(3) {
                let product = ((first + k * 5) % 12) as usize;
                tx.execute(
                    "INSERT OR IGNORE INTO order_items VALUES (?1, ?2, ?3, ?4)",
                    params![id, product as i64 + 1, 1 + rng.next(4), PRODUCTS[product].2],
                )?;
            }
        }
        tx.commit()
    };
    fill().map_err(|e| Error::Other(format!("Could not create the sample database: {e}")))?;
    std::fs::rename(&partial, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sample_has_related_tables_with_data_and_is_created_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("sample.db");
        create(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        let count = |sql: &str| conn.query_row(sql, [], |row| row.get::<_, i64>(0)).unwrap();
        assert_eq!(count("SELECT count(*) FROM customers"), 120);
        assert_eq!(count("SELECT count(*) FROM orders"), 600);
        assert!(count("SELECT count(*) FROM order_items") >= 600);
        assert_eq!(
            count("SELECT count(*) FROM order_items i LEFT JOIN products p ON p.id = i.product_id WHERE p.id IS NULL"),
            0
        );
        assert_eq!(
            count("SELECT count(*) FROM order_totals WHERE total > 0"),
            600
        );
        assert_eq!(
            count("SELECT count(*) FROM orders WHERE json_valid(details)"),
            600
        );

        // A second call leaves the user's (possibly edited) copy alone.
        conn.execute("DELETE FROM order_items", []).unwrap();
        create(&path).unwrap();
        assert_eq!(count("SELECT count(*) FROM order_items"), 0);
    }
}
