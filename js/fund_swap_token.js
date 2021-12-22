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
const tokenAgent = anchor.workspace.TokenAgent
const tokenAgentPK = tokenAgent.programId
//console.log(tokenAgent)

const SPL_ASSOCIATED_TOKEN = new PublicKey('ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL')
async function associatedTokenAddress(walletAddress, tokenMintAddress) {
    const addr = await PublicKey.findProgramAddress(
        [walletAddress.toBuffer(), TOKEN_PROGRAM_ID.toBuffer(), tokenMintAddress.toBuffer()],
        SPL_ASSOCIATED_TOKEN
    )
    const res = { 'pubkey': await addr[0].toString(), 'nonce': addr[1] }
    return res
}

async function programAddress(inputs, programPK = tokenAgentPK) {
    const addr = await PublicKey.findProgramAddress(inputs, programPK)
    const res = { 'pubkey': await addr[0].toString(), 'nonce': addr[1] }
    return res
}

function importSecretKey(keyStr) {
    var dec = new base32.Decoder({ type: "crockford" })
    var spec = dec.write(keyStr).finalize()
    return Keypair.fromSecretKey(new Uint8Array(spec))
}

async function main() {
    var ndjs
    try {
        ndjs = await fs.readFile('../../data/net.json')
    } catch (error) {
        console.error('File Error: ', error)
    }
    const netData = JSON.parse(ndjs.toString())
    //console.log(netData)
    const tokenMint = new PublicKey(netData.tokenMintUSDV)

    const rootKey = await programAddress([tokenAgentPK.toBuffer()])
    const agentToken = await associatedTokenAddress(new PublicKey(rootKey.pubkey), tokenMint)
    const tokenAccount = new PublicKey(agentToken.pubkey)

    console.log('Fund Token: Token Agent PDA (USDV)')
    await tokenAgent.rpc.fundToken(
        agentToken.nonce,
        {
            accounts: {
                ascTokenAccount: SPL_ASSOCIATED_TOKEN,
            },
            remainingAccounts: [
                { pubkey: provider.wallet.publicKey, isWritable: true, isSigner: true },
                { pubkey: tokenMint, isWritable: false, isSigner: false },
                { pubkey: new PublicKey(rootKey.pubkey), isWritable: false, isSigner: false },
                { pubkey: tokenAccount, isWritable: true, isSigner: false },
                { pubkey: TOKEN_PROGRAM_ID, isWritable: false, isSigner: false },
                { pubkey: SystemProgram.programId, isWritable: false, isSigner: false },
                { pubkey: SYSVAR_RENT_PUBKEY, isWritable: false, isSigner: false },
            ]
        }
    )
}

console.log('Begin')
main().then(() => console.log('Success')).catch(error => {
    console.log(error)
})
