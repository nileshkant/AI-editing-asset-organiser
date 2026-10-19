-- v12 (SS-045): gallery name order without a full sort. Matches
-- image_gallery_page ORDER BY lower(relative_path), relative_path, id.
CREATE INDEX asset_registry_gallery ON asset_registry(kind, lower(relative_path), relative_path, id);
