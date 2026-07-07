# CityHall

A Rust API with a React frontend.

## Layout

```
api/    Rust backend (axum)
web/    React frontend (Vite + TypeScript)
```

## Development

### API

```sh
cargo run -p cityhall-api
```

Serves on `http://127.0.0.1:3000`. Health check: `GET /health`.

### Web

```sh
cd web
npm install
npm run dev
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). PR titles follow
[Conventional Commits](https://www.conventionalcommits.org/); a CI check enforces it.

Install the git hooks once with [pre-commit](https://pre-commit.com/):

```sh
pre-commit install
```

## License

MIT. See [LICENSE](LICENSE).
