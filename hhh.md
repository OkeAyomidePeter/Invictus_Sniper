graph TB
    subgraph "Client Layer"
        Mobile[Mobile App<br/>React Native]
        Web[Web Dashboard<br/>Future]
    end

    subgraph "Edge Layer"
        CDN[CDN & Static Assets]
        LB[Load Balancers<br/>Multi-Region]
    end

    subgraph "API Gateway Layer"
        APIGW[API Gateway<br/>Kong/AWS API Gateway]
        AuthGW[Auth Gateway<br/>FastAPI]
    end

    subgraph "Core Services - FastAPI"
        UserSvc[User Service<br/>FastAPI]
        KYCService[KYC/AML Service<br/>FastAPI]
        WalletSvc[Wallet Service<br/>FastAPI]
        PaymentSvc[Payment Service<br/>FastAPI]
        CardSvc[Card Service<br/>FastAPI]
        RewardSvc[Rewards Service<br/>FastAPI]
        NotificationSvc[Notification Service<br/>FastAPI]
    end

    subgraph "High-Performance Services - Rust"
        TxEngine[Transaction Engine<br/>Rust]
        BlockChainSvc[Blockchain Service<br/>Rust]
        SettlementSvc[Settlement Service<br/>Rust]
        PriceFeedSvc[Price Feed Service<br/>Rust]
    end

    subgraph "Event Infrastructure"
        EventBus[Event Bus<br/>Kafka/RabbitMQ]
        EventStore[Event Store]
    end

    subgraph "Data Layer"
        PostgresMain[(PostgreSQL<br/>Primary DB)]
        Redis[(Redis<br/>Cache & Sessions)]
        TimescaleDB[(TimescaleDB<br/>Time-Series)]
        S3Minio[(Object Storage<br/>S3/MinIO)]
    end

    subgraph "External Integrations"
        PaymentRails[Payment Rails<br/>ACH/SWIFT/SEPA/Crypto]
        CardNetworks[Card Networks<br/>Visa/Mastercard]
        KYCProviders[KYC Providers<br/>Onfido/Sumsub]
        BlockchainNodes[Blockchain Nodes<br/>Solana/EVM]
    end

    subgraph "Security & Compliance"
        HSM[HSM<br/>Key Custody]
        AuditLog[Audit Log Service]
        ComplianceSvc[Compliance Service]
    end

    Mobile --> CDN
    Web --> CDN
    CDN --> LB
    LB --> APIGW
    APIGW --> AuthGW
    AuthGW --> UserSvc
    AuthGW --> KYCService
    AuthGW --> WalletSvc
    AuthGW --> PaymentSvc
    AuthGW --> CardSvc
    AuthGW --> RewardSvc

    PaymentSvc --> TxEngine
    WalletSvc --> BlockChainSvc
    PaymentSvc --> SettlementSvc
    RewardSvc --> PriceFeedSvc

    UserSvc --> EventBus
    KYCService --> EventBus
    WalletSvc --> EventBus
    PaymentSvc --> EventBus
    CardSvc --> EventBus
    RewardSvc --> EventBus
    TxEngine --> EventBus
    BlockChainSvc --> EventBus

    EventBus --> EventStore
    EventBus --> NotificationSvc
    EventBus --> AuditLog
    EventBus --> ComplianceSvc

    WalletSvc --> PostgresMain
    UserSvc --> PostgresMain
    PaymentSvc --> PostgresMain
    CardSvc --> PostgresMain
    RewardSvc --> PostgresMain
    KYCService --> PostgresMain

    TxEngine --> Redis
    WalletSvc --> Redis
    PaymentSvc --> Redis

    PriceFeedSvc --> TimescaleDB
    AuditLog --> TimescaleDB

    BlockChainSvc --> HSM
    WalletSvc --> HSM

    PaymentSvc --> PaymentRails
    CardSvc --> CardNetworks
    KYCService --> KYCProviders
    BlockChainSvc --> BlockchainNodes