const { Buffer } = require('buffer')
const { DateTime } = require("luxon")
const { v4: uuidv4, parse: uuidparse } = require('uuid')
const { Keypair, PublicKey, SystemProgram, SYSVAR_RENT_PUBKEY } = require('@solana/web3.js')
const { TOKEN_PROGRAM_ID } = require('@solana/spl-token')
const fs = require('fs').promises
const base32 = require("base32.js")

const anchor = require('@project-serum/anchor')
const provider = anchor.Provider.env()
//const provider = anchor.Provider.local()
anchor.setProvider(provider)

function importSecretKey(keyStr) {
    var dec = new base32.Decoder({ type: "crockford" })
    var spec = dec.write(keyStr).finalize()
    return Keypair.fromSecretKey(new Uint8Array(spec))
}

async function main() {
    var root = importSecretKey('q116ksdz3f4gt7ed10jfmern8r7sj9bqnqqfvc6j1xq72zxgz9be2ptr58n5z1rtktjz2gx1sj0xfxjkjgh70ajyxnjc7fh9v3tfh7r')
    console.log(root.publicKey.toString())
}

main().then(() => process.exit(0)).catch(error => {
    console.log(error)
})
