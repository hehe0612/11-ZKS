import '@nomiclabs/hardhat-waffle';
import '@nomiclabs/hardhat-solpp';
import '@nomiclabs/hardhat-etherscan';
import 'hardhat-typechain';
import 'hardhat-contract-sizer';

const prodConfig = {
    // UPGRADE_NOTICE_PERIOD: 0,
    MAX_AMOUNT_OF_REGISTERED_TOKENS: 127,
    // PRIORITY_EXPIRATION: 101,
    DUMMY_VERIFIER: false
};
const testnetConfig = {
    UPGRADE_NOTICE_PERIOD: 0,
    MAX_AMOUNT_OF_REGISTERED_TOKENS: 127,
    // PRIORITY_EXPIRATION: 101,
    DUMMY_VERIFIER: false
};
const testConfig = {
    UPGRADE_NOTICE_PERIOD: 0,
    MAX_AMOUNT_OF_REGISTERED_TOKENS: 5,
    PRIORITY_EXPIRATION: 101,
    DUMMY_VERIFIER: true
};

const localConfig = Object.assign({}, prodConfig);
localConfig.DUMMY_VERIFIER = process.env.DUMMY_VERIFIER === 'true';

const contractDefs = {
    rinkeby: testnetConfig,
    ropsten: testnetConfig,
    mainnet: prodConfig,
    test: testConfig,
    localhost: localConfig
};

export default {
    solidity: {
        version: '0.7.3',
        settings: {
            optimizer: {
                enabled: true,
                runs: 200
            }
        }
    },
    contractSizer: {
        runOnCompile: false
    },
    paths: {
        sources: './contracts'
    },
    solpp: {
        defs: (() => {
            if (process.env.CONTRACT_TESTS) {
                return contractDefs.test;
            }
            return contractDefs[process.env.ETH_NETWORK];
        })()
    },
    networks: {
        env: {
            url: process.env.WEB3_URL
        },
        hardhat: {
            allowUnlimitedContractSize: true
        }
    },
    etherscan: {
        apiKey: process.env.ETHERSCAN_API_KEY
    }
};
