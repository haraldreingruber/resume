#!/usr/bin/env sh
# Builds the PDF resume. Content comes from ../content/resume.yaml via
# generated/resume-data.tex -- run `cargo gen latex` after editing the YAML.
cd "$(dirname "$0")" && pdflatex -interaction=nonstopmode -halt-on-error -jobname=Harald_Reingruber_resume main.tex
