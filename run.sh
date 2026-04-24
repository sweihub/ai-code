#!/bin/bash

while read i; do 
    (
        echo "translate $i"
        opencode run "follow ./CLAUDE.md, continue to translate the $i into our Rust project, translate the logics completely, line by line, fix any errors, DO NOT MISS ANYTHING!"
    ) < /dev/null;
        exit;
done <./list.txt
