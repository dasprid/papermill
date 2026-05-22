CREATE TABLE uploaded_invoices (
    source_id TEXT NOT NULL,
    external_id TEXT NOT NULL,
    paperless_task_id TEXT NOT NULL,
    invoice_issued_at TEXT NOT NULL,
    uploaded_at TEXT NOT NULL,
    PRIMARY KEY (source_id, external_id)
);

CREATE INDEX idx_uploaded_invoices_source_date
    ON uploaded_invoices (source_id, invoice_issued_at);
