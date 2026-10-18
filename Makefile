.PHONY: serve_doc

serve_doc:
	cargo doc --no-deps
	python3 -m http.server 8000 --directory target/doc
