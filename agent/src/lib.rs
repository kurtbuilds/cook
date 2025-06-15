// you want to be able to run this in multiple contexts:
// local agent - runs commands using rust APIs
// ssh agent - runs commands over ssh
//
// and then in terms of deployment, you want the cook command
// to be able to either:
// by default, you run commands over ssh (ssh agent)
// eventually, you turn on a config that deploys the agent on the remote (can be done via ssh)
// thereafter, the ssh agent checks if the agent exists on the remote
// cook apply
pub mod local;
