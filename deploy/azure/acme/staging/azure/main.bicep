// Azure Bicep for tenant acme (staging)
param tenant string = 'acme'
param environment string = 'staging'
param telemetryEndpoint string = 'https://otel.greentic.ai'
param natsAdminUrl string = 'https://nats.staging.acme.svc'
param secretPaths object = {}
var deploymentName = '\${tenant}-\${environment}'
var telemetryAttributes = 'deployment.environment=staging,greentic.tenant=acme,service.name=greentic-deployer-azure'

resource runnergreentic_acme_runner 'Microsoft.Web/containerApps@2023-08-01' = {
  name: '${deploymentName}-greentic_acme_runner'
  location: resourceGroup().location
  properties: {
    configuration: {
      secrets:
      [
        { name: 'SLACK_BOT_TOKEN', value: secretPaths['SLACK_BOT_TOKEN'] }
        { name: 'TEAMS_CLIENT_SECRET', value: secretPaths['TEAMS_CLIENT_SECRET'] }
      ]
    }
    template: {
      containers: [
        {
          name: 'greentic_acme_runner'
          image: 'greentic/runner:latest'
          env: [
          { name: 'NATS_URL', value: natsAdminUrl }
          { name: 'OTEL_EXPORTER_OTLP_ENDPOINT', value: telemetryEndpoint }
          { name: 'OTEL_RESOURCE_ATTRIBUTES', value: 'deployment.environment=staging,greentic.tenant=acme,service.name=greentic-deployer-azure' }
          { name: 'SLACK_BOT_TOKEN', secretRef: 'SLACK_BOT_TOKEN' }
          { name: 'TEAMS_CLIENT_SECRET', secretRef: 'TEAMS_CLIENT_SECRET' }
          ]
        }
      ]
    }
  }
}


// OAuth redirect URLs
// - /oauth/microsoft/callback -> /oauth/microsoft/callback/acme/staging
// - /oauth/slack/callback -> /oauth/slack/callback/acme/staging
