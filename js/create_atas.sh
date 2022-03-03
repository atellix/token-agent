#!/bin/bash

ROOT=/Users/mfrager/Build/solana/data

KEY_TOKEN_USDV=$ROOT/export/key-usdv-token-1.json

TOKEN_USDV=$(solana-keygen pubkey $KEY_TOKEN_USDV)
TOKEN_AGENT_ROOT=$(node print_root.js)

# Token Agent / USDV
echo 'ATA: Token Agent USDV'
spl-token create-account $TOKEN_USDV --owner $TOKEN_AGENT_ROOT
