const anchor = require('@project-serum/anchor')
const provider = anchor.Provider.env()
anchor.setProvider(provider)

async function main() {
    let sig = process.argv[2]
    let pt = await provider.connection.getParsedTransaction(sig, 'confirmed')
    console.log(pt)
}

main()
