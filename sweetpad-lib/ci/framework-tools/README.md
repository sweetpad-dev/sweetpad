# Tools for the hybrid fixtures

The npm lock pins Node 24.22.2 and Bun 1.3.14.
Copy both JSON files into a working tool directory, then run `npm ci` there.
Pass its `node_modules/.bin` directory to `linux-project-prepare.py --tool-bin`.

The default directory is `corpus/_linux-followups/tools/js`.
The recipe also requires Bundler 4.0.20 and a Ruby version accepted by the project Gemfiles.
Flutter uses the separate revision recorded in the preparation script.
