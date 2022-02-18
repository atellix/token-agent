const { PublicKey } = require('@solana/web3.js')
const anchor = require('@project-serum/anchor')
const provider = anchor.Provider.env()
anchor.setProvider(provider)

const tokenAgent = anchor.workspace.TokenAgent

async function main() {
    var subscrData = new PublicKey('ARARDkE3QACJTuwSmsg4uRFaVDz454ETfT9xUGbwi9xB')
    var act = await tokenAgent.account.subscrData.fetch(subscrData)
    console.log(act)
}

main()
