# Complex

One language for the whole web application: database, server and browser in
one source, with a compiler that refuses unsafe code.

This repository holds the first runtime, written in Rust with no external
crates, and the source of [complex.robertblok.com](https://complex.robertblok.com),
which is written in Complex itself.

## Try it

    cargo build --release
    ./target/release/complex check site
    ./target/release/complex serve site          # http://127.0.0.1:8080/

## What works in v0.1

- `let`, `component`, `page` with URL parameters (`/work/:slug`)
- views that compile to HTML: elements, attributes, `for`, `if`/`else`, components with `children`
- a type checker: records, lists, `Text`, `Int`, `Bool`, with error messages written for humans
- information flow control: a `Secret<T>` value cannot reach a view, an attribute,
  a branch or a loop; the only way out is an explicit `masked(value, n)`
- automatic HTML escaping, no `javascript:` URLs, no path traversal
- an HTTP/1.1 server with header and body limits and read timeouts
- a CGI mode for shared hosting

## Commands

    complex check  <dir>          parse and type-check, run nothing
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

Shared hosting (CGI): copy `complex`, `site/` and `.htaccess` to the web root;
every request runs `complex cgi site`.
