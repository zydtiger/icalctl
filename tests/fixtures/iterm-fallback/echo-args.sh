#!/bin/zsh

for argument in "$@"; do
    print -r -- "ARG:${argument}"
done
