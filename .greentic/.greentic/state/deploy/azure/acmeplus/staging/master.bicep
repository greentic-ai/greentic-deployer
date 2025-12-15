// Azure Bicep for tenant acmeplus (staging)
param tenant string = 'acmeplus'
param environment string = 'staging'
param telemetryEndpoint string = 'https://otel.greentic.ai'
param natsAdminUrl string = 'https://nats.staging.acmeplus.svc'
param secretPaths object = {}
var deploymentName = '\${tenant}-\${environment}'
var telemetryAttributes = 'deployment.environment=staging,greentic.tenant=acmeplus,service.name=greentic-deployer-azure'

resource runnergreentic_acme_plus_component 'Microsoft.Web/containerApps@2023-08-01' = {
  name: '${deploymentName}-greentic_acme_plus_component'
  location: resourceGroup().location
  properties: {
    configuration: {
      secrets: []
    }
    template: {
      scale: { minReplicas: 2, maxReplicas: 2 }
      containers: [
        {
          name: 'greentic_acme_plus_component'
          image: 'greentic/runner:latest'
          env: [
          { name: 'NATS_URL', value: natsAdminUrl }
          { name: 'OTEL_EXPORTER_OTLP_ENDPOINT', value: telemetryEndpoint }
          { name: 'OTEL_RESOURCE_ATTRIBUTES', value: 'deployment.environment=staging,greentic.tenant=acmeplus,service.name=greentic-deployer-azure' }
          ]
          resources: { requests: { cpu: '0.50', memory: '1.00Gi' } }
        }
      ]
    }
  }
}

