// Azure Bicep for tenant acmeplus (staging)
param tenant string = 'acmeplus'
param environment string = 'staging'
param telemetryEndpoint string = 'https://otel.greentic.ai'
param natsAdminUrl string = 'https://nats.staging.acmeplus.svc'
param secretPaths object = {}
var deploymentName = '\${tenant}-\${environment}'
var telemetryAttributes = 'deployment.environment=staging,greentic.tenant=acmeplus,service.name=greentic-deployer-azure'

resource runnergreentic_acme_plus_runner 'Microsoft.Web/containerApps@2023-08-01' = {
  name: '${deploymentName}-greentic_acme_plus_runner'
  location: resourceGroup().location
  properties: {
    configuration: {
      secrets:
      [
        { name: 'SLACK_BOT_TOKEN', value: secretPaths['SLACK_BOT_TOKEN'] }
        { name: 'CRM_API_TOKEN', value: secretPaths['CRM_API_TOKEN'] }
        { name: 'PAGERDUTY_API_KEY', value: secretPaths['PAGERDUTY_API_KEY'] }
        { name: 'TEAMS_CLIENT_SECRET', value: secretPaths['TEAMS_CLIENT_SECRET'] }
      ]
    }
    template: {
      containers: [
        {
          name: 'greentic_acme_plus_runner'
          image: 'greentic/runner:latest'
          env: [
          { name: 'NATS_URL', value: natsAdminUrl }
          { name: 'OTEL_EXPORTER_OTLP_ENDPOINT', value: telemetryEndpoint }
          { name: 'OTEL_RESOURCE_ATTRIBUTES', value: 'deployment.environment=staging,greentic.tenant=acmeplus,service.name=greentic-deployer-azure' }
          { name: 'SLACK_BOT_TOKEN', secretRef: 'SLACK_BOT_TOKEN' }
          { name: 'CRM_API_TOKEN', secretRef: 'CRM_API_TOKEN' }
          { name: 'PAGERDUTY_API_KEY', secretRef: 'PAGERDUTY_API_KEY' }
          { name: 'TEAMS_CLIENT_SECRET', secretRef: 'TEAMS_CLIENT_SECRET' }
          ]
        }
      ]
    }
  }
}


// Channel ingress endpoints
// - Slack Support (type = channels.slack.support, oauth_required = false)
//   ingress: https://deploy.greentic.ai/ingress/staging/acmeplus/channels.slack.support
// - Teams Ops (type = channels.teams.ops, oauth_required = false)
//   ingress: https://deploy.greentic.ai/ingress/staging/acmeplus/channels.teams.ops
// - messaging.subjects.ops.alerts (type = messaging.subjects.ops.alerts, oauth_required = false)
//   ingress: https://deploy.greentic.ai/ingress/staging/acmeplus/messaging.subjects.ops.alerts
// - messaging.subjects.support.inbound (type = messaging.subjects.support.inbound, oauth_required = false)
//   ingress: https://deploy.greentic.ai/ingress/staging/acmeplus/messaging.subjects.support.inbound

// OAuth redirect URLs
// - /oauth/microsoft/callback -> /oauth/microsoft/callback/acmeplus/staging
// - /oauth/slack/callback -> /oauth/slack/callback/acmeplus/staging
