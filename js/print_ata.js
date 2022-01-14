const { PublicKey } = require('@solana/web3.js')
const { associatedTokenAddress } = require('../../js/atellix-common')

async function main() {
    let mint = new PublicKey(process.argv[2])
    let owner = new PublicKey(process.argv[3])
    let ata = await associatedTokenAddress(owner, mint)
    console.log(ata.pubkey)
}

main()
