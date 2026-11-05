# Complex

One language for the whole web application: database, server and browser in
one source, with a compiler that refuses unsafe code.

This repository holds the first runtime, written in Rust with no external
crates, and the source of [complex.robertblok.com](https://complex.robertblok.com),
which is written in Complex itself. The site is about the language only.

## Try it

    cargo build --release
    ./target/release/complex check site
    ./target/release/complex serve site          # http://127.0.0.1:8080/

## What works in v0.2

- `let`, `component`, `page` with URL parameters (`/work/:slug`)
- views that compile to HTML: elements, attributes, `for`, `if`/`else`, components with `children`
- a type checker: records, lists, `Text`, `Int`, `Bool`, with error messages written for humans
- `model` with labelled fields (`@personal`, `@secret`) and `seed` data
- information flow control over a three-point lattice (public < personal < secret):
  a labelled value cannot reach a view, an attribute, a branch, a loop or a `log` line,
  and anything computed from it carries its label; the only way across is an explicit
  `masked(value, n)` or `reveal(value)`
- `complex audit`: every labelled field and every declassification, with file and line,
  derived from the source by the compiler
- automatic HTML escaping, no `javascript:` URLs, no path traversal
- an HTTP/1.1 server with header and body limits and read timeouts
- a CGI mode for shared hosting

## Commands

    complex check  <dir>          parse and type-check, run nothing
    complex audit  <dir>          labelled fields, declassifications, sinks
    complex parse  <file>         print the syntax tree
    complex render <dir> <path>   print the HTML for one page
    complex serve  <dir> [addr]   development server
    complex cgi    <dir>          answer one request as a CGI program

## Layout

    src/lexer.rs    text -> tokens
    src/parser.rs   tokens -> syntax tree (recursive descent)
    src/check.rs    names, types, information flow
    src/interp.rs   syntax tree -> HTML (tree-walking interpreter)
    src/http.rs     HTTP/1.1 on std::net
    site/           the portfolio, in .complex
    site/public/    static files

## Deploy

Development: `complex serve site 127.0.0.1:8080` behind Caddy or Nginx for HTTPS.

Note: when Caddy runs in Docker with the Caddyfile bind-mounted, edit it in place
(not with `sed -i`, which creates a new inode) or `docker restart caddy` afterwards.

Shared hosting (CGI): copy `complex`, `site/` and `.htaccess` to the web root;
every request runs `complex cgi site`.
